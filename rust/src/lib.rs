//! Rust reference engine for the AttoLang Redis clone.
//! On-disk layout is experimental and NOT compatible with Redis RDB/AOF.
//! Uses only std; the first implementation is intended for Unix hosts.
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod server;

const WAL_MAGIC: [u8; 4] = *b"AWL1";
const SST_MAGIC: [u8; 4] = *b"AST1";
const MAN_MAGIC: [u8; 4] = *b"AMF1";
const HEADER: usize = 12;
const RECORD: usize = 17;
const MAX_MANIFEST: usize = 8 * 1024 * 1024;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}
fn blocked(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, msg.into())
}
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}
fn crc32(input: &[u8]) -> u32 {
    let mut crc: u32 = !0;
    for &b in input {
        crc ^= b as u32;
        for _ in 0..8 { crc = (crc >> 1) ^ (if crc & 1 != 0 { 0xedb8_8320 } else { 0 }); }
    }
    !crc
}
fn put_u16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_be_bytes()); }
fn put_u32(v: &mut Vec<u8>, x: u32) { v.extend_from_slice(&x.to_be_bytes()); }
fn put_u64(v: &mut Vec<u8>, x: u64) { v.extend_from_slice(&x.to_be_bytes()); }
struct Cursor<'a> { b: &'a [u8], at: usize }
impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Self { Self { b, at: 0 } }
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or_else(|| invalid("integer overflow"))?;
        if end > self.b.len() { return Err(invalid("truncated binary data")); }
        let out = &self.b[self.at..end]; self.at = end; Ok(out)
    }
    fn u16(&mut self) -> io::Result<u16> { Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> io::Result<u32> { Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> io::Result<u64> { Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap())) }
    fn done(&self) -> io::Result<()> { if self.at == self.b.len() { Ok(()) } else { Err(invalid("trailing data")) } }
}
fn frame(magic: [u8;4], payload: &[u8]) -> io::Result<Vec<u8>> {
    let n = u32::try_from(payload.len()).map_err(|_| invalid("frame exceeds u32"))?;
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(&magic); put_u32(&mut out, n); put_u32(&mut out, crc32(payload));
    out.extend_from_slice(payload); Ok(out)
}
fn frame_record(magic: [u8; 4], seq: u64, key: &[u8], value: Option<&[u8]>) -> io::Result<Vec<u8>> {
    let mut payload = Vec::with_capacity(RECORD + key.len() + value.map_or(0, |v| v.len()));
    put_u64(&mut payload, seq);
    put_u32(&mut payload, u32::try_from(key.len()).map_err(|_| invalid("key too large"))?);
    put_u32(&mut payload, u32::try_from(value.map_or(0, |v| v.len())).map_err(|_| invalid("value too large"))?);
    payload.push(if value.is_some() { 1 } else { 2 });
    payload.extend_from_slice(key);
    if let Some(v) = value { payload.extend_from_slice(v); }
    frame(magic, &payload)
}
#[derive(Clone, Debug)]
struct Entry { seq: u64, value: Option<Vec<u8>> }
fn decode_record(payload: &[u8], cfg: &Config) -> io::Result<(u64, Vec<u8>, Option<Vec<u8>>)> {
    let mut r = Cursor::new(payload);
    let seq = r.u64()?;
    let nk = r.u32()? as usize;
    let nv = r.u32()? as usize;
    let op = r.take(1)?[0];
    if nk == 0 || nk > cfg.max_key_bytes || nv > cfg.max_value_bytes ||
        (op != 1 && op != 2) || (op == 2 && nv != 0) || seq == 0 {
        return Err(invalid("invalid record header"));
    }
    let key = r.take(nk)?.to_vec();
    let value = r.take(nv)?;
    r.done()?;
    Ok((seq, key, if op == 2 { None } else { Some(value.to_vec()) }))
}
fn scan(path: &Path, magic: [u8;4], cfg: &Config, salvage: bool,
        mut each: impl FnMut(u64, u64, Vec<u8>, Option<Vec<u8>>) -> io::Result<()>) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(salvage).open(path)?;
    let mut last_seq = 0u64;
    let mut last_key: Option<Vec<u8>> = None;
    let max_frame = RECORD.checked_add(cfg.max_key_bytes)
        .and_then(|x| x.checked_add(cfg.max_value_bytes)).ok_or_else(|| invalid("frame size overflow"))?;
    loop {
        let offset = file.stream_position()?;
        let mut hdr = [0; HEADER];
        // read() may short-read on healthy files. Only an actual EOF
        // indicates a torn frame header.
        let mut n = 0;
        while n < HEADER {
            let read = file.read(&mut hdr[n..])?;
            if read == 0 { break; }
            n += read;
        }
        if n == 0 { return Ok(()); }
        if n != HEADER {
            if salvage { file.set_len(offset)?; file.sync_all()?; return Ok(()); }
            return Err(invalid("truncated frame header"));
        }
        if hdr[..4] != magic { return Err(invalid("bad magic")); }
        let size = u32::from_be_bytes(hdr[4..8].try_into().unwrap()) as usize;
        if !(RECORD..=max_frame).contains(&size) { return Err(invalid("invalid frame length")); }
        let expected = u32::from_be_bytes(hdr[8..12].try_into().unwrap());
        let mut payload = vec![0; size];
        if let Err(e) = file.read_exact(&mut payload) {
            if e.kind() == io::ErrorKind::UnexpectedEof && salvage {
                file.set_len(offset)?; file.sync_all()?; return Ok(());
            }
            return Err(e);
        }
        if crc32(&payload) != expected { return Err(invalid("frame checksum mismatch")); }
        let (seq, key, value) = decode_record(&payload, cfg)?;
        if magic == WAL_MAGIC {
            if seq <= last_seq { return Err(invalid("out-of-order WAL LSN")); }
            last_seq = seq;
        } else if let Some(prev) = &last_key {
            if key <= *prev { return Err(invalid("unsorted/duplicate SST key")); }
        }
        if magic == SST_MAGIC { last_key = Some(key.clone()); }
        each(offset, seq, key, value)?;
    }
}
#[derive(Clone, Debug)]
pub struct Config {
    pub shards: usize,
    pub spill_bytes: usize,
    pub pending_limit: usize,
    pub max_key_bytes: usize,
    pub max_value_bytes: usize,
    pub fsync_every_second: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self { shards:64, spill_bytes:64*1024*1024, pending_limit:4,
               max_key_bytes:1024*1024, max_value_bytes:64*1024*1024,
               fsync_every_second:false }
    }
}
impl Config {
    fn validate(&self) -> io::Result<()> {
        if !(1..=256).contains(&self.shards) || self.spill_bytes == 0 || self.pending_limit == 0
           || self.max_key_bytes == 0 || self.max_value_bytes > u32::MAX as usize
           || self.max_key_bytes > u32::MAX as usize
           || RECORD.checked_add(self.max_key_bytes).and_then(|v|v.checked_add(self.max_value_bytes)).is_none() {
            return Err(invalid("invalid configuration"));
        }
        Ok(())
    }
}
fn segment_filename(idx: usize, name: &str) -> bool {
    let prefix = format!("shard-{idx:03}-");
    name.starts_with(&prefix) && name.ends_with(".sst") &&
    name.len() == prefix.len() + 32 + 4 &&
    name[prefix.len()..prefix.len()+32].bytes().all(|c|c.is_ascii_hexdigit())
}
#[derive(Clone)]
struct Manifest { fence: u64, marks: Vec<u64>, files: Vec<Vec<String>> }
impl Manifest {
    fn fresh(shards: usize) -> Self {
        Self { fence: 0, marks: vec![0;shards], files: vec![Vec::new(); shards] }
    }
    fn encode(&self) -> io::Result<Vec<u8>> {
        let mut p = Vec::new();
        put_u32(&mut p, u32::try_from(self.marks.len()).map_err(|_|invalid("shard count"))?);
        put_u64(&mut p, self.fence);
        for (idx, names) in self.files.iter().enumerate() {
            put_u64(&mut p, self.marks[idx]);
            put_u32(&mut p, u32::try_from(names.len()).map_err(|_| invalid("manifest too large"))?);
            for name in names {
                if !segment_filename(idx, name) { return Err(invalid("unsafe segment name")); }
                put_u16(&mut p, u16::try_from(name.len()).map_err(|_| invalid("name length"))?);
                p.extend_from_slice(name.as_bytes());
            }
        }
        if p.len() > MAX_MANIFEST { return Err(invalid("manifest too large")); }
        frame(MAN_MAGIC, &p)
    }
    fn save(&self, dir: &Path) -> io::Result<()> {
        let tmp = dir.join("MANIFEST.tmp");
        let bytes = self.encode()?;
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&bytes)?; f.sync_all()?;
        }
        fs::rename(&tmp, dir.join("MANIFEST"))?;
        sync_dir(dir)
    }
    fn load(dir: &Path, shards: usize) -> io::Result<Self> {
        let path = dir.join("MANIFEST");
        if !path.exists() {
            if dir.join("FORMAT.v1").exists() ||
                dir.read_dir()?.any(|x| x.map(|d| {
                    d.file_name().to_string_lossy().starts_with("shard-") ||
                    d.file_name() == "WAL"
                }).unwrap_or(true)) {
                return Err(invalid("missing manifest from existing database"));
            }
            let out = Self::fresh(shards);
            out.save(dir)?;
            let mut marker = File::create(dir.join("FORMAT.v1"))?;
            marker.write_all(b"atto-redis-rust-v1\n")?; marker.sync_all()?;
            sync_dir(dir)?;
            return Ok(out);
        }
        if path.symlink_metadata()?.file_type().is_symlink() { return Err(invalid("symlink manifest")); }
        let marker = dir.join("FORMAT.v1");
        if marker.symlink_metadata()?.file_type().is_symlink() || fs::read(&marker)? != b"atto-redis-rust-v1\n" {
            return Err(invalid("bad format marker"));
        }
        let b = fs::read(path)?;
        if b.len() < HEADER || b.len() > MAX_MANIFEST + HEADER || b[..4] != MAN_MAGIC {
            return Err(invalid("bad manifest header"));
        }
        let len = u32::from_be_bytes(b[4..8].try_into().unwrap()) as usize;
        if len + HEADER != b.len() || crc32(&b[HEADER..]) != u32::from_be_bytes(b[8..12].try_into().unwrap()) {
            return Err(invalid("manifest checksum/length"));
        }
        let mut r = Cursor::new(&b[HEADER..]);
        if r.u32()? as usize != shards { return Err(invalid("different shard topology")); }
        let fence = r.u64()?;
        let mut marks = Vec::with_capacity(shards);
        let mut files = Vec::with_capacity(shards);
        for idx in 0..shards {
            let watermark = r.u64()?;
            if watermark < fence { return Err(invalid("invalid shard watermark")); }
            let count = r.u32()? as usize;
            if count > MAX_MANIFEST / 48 { return Err(invalid("too many segments")); }
            let mut names = Vec::with_capacity(count);
            for _ in 0..count {
                let name_len = r.u16()? as usize;
                let name = std::str::from_utf8(r.take(name_len)?).map_err(|_|invalid("segment name UTF8"))?.to_string();
                if !segment_filename(idx, &name) || names.contains(&name) { return Err(invalid("invalid manifest segment")); }
                names.push(name);
            }
            marks.push(watermark); files.push(names);
        }
        r.done()?;
        Ok(Self { fence, marks, files })
    }
}
#[cfg(unix)]
fn lock_store(dir: &Path) -> io::Result<File> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" { fn flock(fd: i32, operation: i32) -> i32; }
    let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dir.join("WAL.lock"))?;
    if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 { return Err(blocked("store already opened")); }
    Ok(file)
}
#[cfg(not(unix))]
fn lock_store(_dir: &Path) -> io::Result<File> { Err(invalid("Rust oracle requires Unix flock")) }

struct Wal { path: PathBuf, f: File, seq: u64, poisoned: bool, last_sync: Instant, every_second: bool }
impl Wal {
    fn open(dir: &Path, cfg: &Config) -> io::Result<Self> {
        let path = dir.join("WAL");
        if path.exists() && path.symlink_metadata()?.file_type().is_symlink() { return Err(invalid("symlink WAL")); }
        let f = OpenOptions::new().create(true).read(true).append(true).open(&path)?;
        sync_dir(dir)?;
        let mut seq = 0;
        scan(&path, WAL_MAGIC, cfg, true, |_,n,_,_| { seq=n; Ok(()) })?;
        Ok(Self { path, f, seq, poisoned:false, last_sync:Instant::now(), every_second:cfg.fsync_every_second })
    }
    fn append(&mut self, key: &[u8], value: Option<&[u8]>) -> io::Result<u64> {
        if self.poisoned { return Err(invalid("WAL poisoned")); }
        let action = (|| {
            let next = self.seq.checked_add(1).ok_or_else(|| invalid("LSN overflow"))?;
            let b = frame_record(WAL_MAGIC, next, key, value)?;
            self.f.write_all(&b)?;
            if !self.every_second || self.last_sync.elapsed() >= Duration::from_secs(1) {
                self.f.sync_data()?;
                self.last_sync = Instant::now();
            }
            self.seq = next;
            Ok(next)
        })();
        if action.is_err() { self.poisoned = true; }
        action
    }
    fn prune(&mut self, through: u64, cfg: &Config) -> io::Result<()> {
        if self.poisoned { return Err(invalid("WAL poisoned")); }
        // A global ordering gate is held by the caller across prune/append.
        let temp = self.path.with_extension("tmp");
        let work = (|| {
            let mut out = File::create(&temp)?;
            scan(&self.path, WAL_MAGIC, cfg, false, |_,seq,key,value| {
                if seq > through { out.write_all(&frame_record(WAL_MAGIC,seq,&key,value.as_deref())?)?; }
                Ok(())
            })?;
            out.sync_all()?;
            fs::rename(&temp, &self.path)?;
            sync_dir(self.path.parent().unwrap())?;
            self.f = OpenOptions::new().read(true).append(true).open(&self.path)?;
            Ok(())
        })();
        if work.is_err() { self.poisoned = true; }
        work
    }
}
#[derive(Clone)]
struct Segment { name: String, path: PathBuf, index: BTreeMap<Vec<u8>, u64> }
impl Segment {
    fn load(path: PathBuf, cfg: &Config) -> io::Result<Self> {
        if path.symlink_metadata()?.file_type().is_symlink() { return Err(invalid("symlink SST")); }
        let name = path.file_name().ok_or_else(||invalid("segment filename"))?.to_string_lossy().to_string();
        let mut index = BTreeMap::new();
        scan(&path, SST_MAGIC, cfg, false, |offset,_,key,_| { index.insert(key,offset); Ok(()) })?;
        Ok(Self { name, path, index })
    }
    fn write(dir: &Path, idx: usize, data: &BTreeMap<Vec<u8>,Entry>, cfg: &Config) -> io::Result<Self> {
        let serial = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_|invalid("clock"))?.as_nanos() as u64;
        let name = format!("shard-{idx:03}-{nanos:016x}{serial:016x}.sst");
        let path = dir.join(&name);
        let tmp = dir.join(format!("{name}.tmp"));
        if path.exists() { return Err(invalid("segment filename collision")); }
        {
            let mut f = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
            for (key,v) in data { f.write_all(&frame_record(SST_MAGIC, v.seq, key, v.value.as_deref())?)?; }
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?; sync_dir(dir)?;
        Self::load(path,cfg)
    }
    fn get(&self, key: &[u8], cfg: &Config) -> io::Result<Option<Entry>> {
        let Some(&offset) = self.index.get(key) else { return Ok(None); };
        let mut f = File::open(&self.path)?;
        f.seek(SeekFrom::Start(offset))?;
        let mut hdr = [0;HEADER]; f.read_exact(&mut hdr)?;
        let size = u32::from_be_bytes(hdr[4..8].try_into().unwrap()) as usize;
        let max = RECORD.checked_add(cfg.max_key_bytes).and_then(|v|v.checked_add(cfg.max_value_bytes))
            .ok_or_else(|| invalid("frame limit overflow"))?;
        if hdr[..4] != SST_MAGIC || size < RECORD || size > max { return Err(invalid("bad SST frame")); }
        let mut data = vec![0;size]; f.read_exact(&mut data)?;
        if crc32(&data) != u32::from_be_bytes(hdr[8..12].try_into().unwrap()) { return Err(invalid("corrupt SST")); }
        let (seq, found, val) = decode_record(&data, cfg)?;
        if found != key { return Err(invalid("SST index mismatch")); }
        Ok(Some(Entry{seq,value:val}))
    }
}
#[derive(Default)]
struct Shard {
    active: HashMap<Vec<u8>,Entry>, active_bytes: usize,
    pending: VecDeque<Arc<BTreeMap<Vec<u8>, Entry>>>,
    segments: Vec<Segment>,
}
struct Shared {
    dir: PathBuf, cfg: Config, gate: Mutex<()>,
    wal: Mutex<Wal>, manifest: Mutex<Manifest>, shards: Vec<Mutex<Shard>>,
    failed: Mutex<Option<String>>, ready: Condvar,
}
enum Job {
    Flush(usize, Arc<BTreeMap<Vec<u8>,Entry>>),
    Barrier(mpsc::Sender<()>),
    Exit,
}
pub struct Engine {
    shared: Arc<Shared>, jobs: mpsc::Sender<Job>,
    worker: Mutex<Option<JoinHandle<()>>>, _lease: File,
}
fn store_failure(shared: &Shared, error: io::Error) {
    *shared.failed.lock().unwrap() = Some(error.to_string());
    shared.ready.notify_all();
}
fn flush_worker(shared: Arc<Shared>, rx: mpsc::Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        match job {
            Job::Exit => break,
            Job::Barrier(reply) => { let _ = reply.send(()); }
            Job::Flush(idx,generation) => {
                if shared.failed.lock().unwrap().is_some() { continue; }
                let result = (|| -> io::Result<()> {
                    let segment = Segment::write(&shared.dir, idx, &generation, &shared.cfg)?;
                    let mut shard = shared.shards[idx].lock().unwrap();
                    if !shard.pending.front().is_some_and(|v|Arc::ptr_eq(v,&generation)) {
                        return Err(invalid("out-of-order memtable flush"));
                    }
                    let mut man = shared.manifest.lock().unwrap();
                    man.files[idx].push(segment.name.clone());
                    let max_seq = generation.values().map(|x|x.seq).max().unwrap_or(0);
                    man.marks[idx] = man.marks[idx].max(max_seq);
                    man.save(&shared.dir)?;
                    shard.segments.push(segment);
                    shard.pending.pop_front();
                    shared.ready.notify_all();
                    Ok(())
                })();
                if let Err(e) = result { store_failure(&shared,e); }
            }
        }
    }
}
impl Engine {
    pub fn config(&self) -> Config { self.shared.cfg.clone() }
    pub fn open(path: impl AsRef<Path>, cfg: Config) -> io::Result<Self> {
        cfg.validate()?;
        let dir=path.as_ref().to_path_buf(); fs::create_dir_all(&dir)?;
        let lease=lock_store(&dir)?;
        let man=Manifest::load(&dir,cfg.shards)?;
        let mut shards: Vec<Mutex<Shard>>=Vec::with_capacity(cfg.shards);
        for i in 0..cfg.shards {
            let mut shard=Shard::default();
            for name in &man.files[i] { shard.segments.push(Segment::load(dir.join(name),&cfg)?); }
            shards.push(Mutex::new(shard));
        }
        let mut wal=Wal::open(&dir,&cfg)?;
        wal.seq = wal.seq.max(*man.marks.iter().max().unwrap_or(&0));
        scan(&wal.path,WAL_MAGIC,&cfg,false,|_,seq,key,value| {
            let idx=crc32(&key) as usize % cfg.shards;
            if seq > man.marks[idx] {
                let shard=shards[idx].get_mut().map_err(|_|invalid("poisoned shard"))?;
                let old=shard.active.insert(key.clone(),Entry{seq,value:value.clone()});
                shard.active_bytes=shard.active_bytes
                    .saturating_add(key.len() + value.as_ref().map_or(0,Vec::len)+48)
                    .saturating_sub(old.as_ref().map_or(0,|e|key.len()+e.value.as_ref().map_or(0,Vec::len)+48));
            }
            Ok(())
        })?;
        let shared=Arc::new(Shared { dir, cfg, gate:Mutex::new(()),wal:Mutex::new(wal),
                                   manifest:Mutex::new(man),shards,failed:Mutex::new(None),
                                   ready:Condvar::new() });
        let (tx,rx)=mpsc::channel();
        let worker_shared=Arc::clone(&shared);
        let worker=thread::spawn(move||flush_worker(worker_shared,rx));
        Ok(Self { shared,jobs:tx,worker:Mutex::new(Some(worker)),_lease:lease })
    }
    fn healthy(&self) -> io::Result<()> {
        if let Some(err)=self.shared.failed.lock().unwrap().as_ref() {
            return Err(invalid(format!("background flush failed: {err}")));
        }
        if self.shared.wal.lock().unwrap().poisoned { return Err(invalid("WAL poisoned")); }
        Ok(())
    }
    fn index(&self,key:&[u8])->usize { crc32(key) as usize % self.shared.cfg.shards }
    fn validate(&self,key:&[u8],value:Option<&[u8]>)->io::Result<()> {
        if key.is_empty() || key.len()>self.shared.cfg.max_key_bytes ||
           value.is_some_and(|v|v.len()>self.shared.cfg.max_value_bytes) {
            return Err(invalid("key or value outside configured limits"));
        }
        Ok(())
    }
    fn rotate(&self, idx: usize, shard:&mut Shard) -> io::Result<()> {
        if shard.active.is_empty() { return Ok(()); }
        let generation: Arc<BTreeMap<Vec<u8>,Entry>>=Arc::new(std::mem::take(&mut shard.active).into_iter().collect());
        shard.active_bytes=0;
        shard.pending.push_back(Arc::clone(&generation));
        self.jobs.send(Job::Flush(idx,generation)).map_err(|_|invalid("flush worker unavailable"))
    }
    fn lookup_locked(&self, shard:&Shard,key:&[u8])->io::Result<Option<Vec<u8>>> {
        if let Some(e)=shard.active.get(key) { return Ok(e.value.clone()); }
        for p in shard.pending.iter().rev() { if let Some(e)=p.get(key) { return Ok(e.value.clone()); } }
        for s in shard.segments.iter().rev() { if let Some(e)=s.get(key,&self.shared.cfg)? { return Ok(e.value); } }
        Ok(None)
    }
    fn write(&self,key:&[u8],value:Option<&[u8]>,conditional_delete:bool)->io::Result<(u64,bool)> {
        self.validate(key,value)?;
        let _gate=self.shared.gate.lock().unwrap();
        self.healthy()?;
        let idx=self.index(key);
        let mut shard=self.shared.shards[idx].lock().unwrap();
        if shard.pending.len()>=self.shared.cfg.pending_limit { return Err(blocked("immutable memtable backlog")); }
        let existed=if conditional_delete { self.lookup_locked(&shard,key)?.is_some() } else { false };
        if conditional_delete && !existed { return Ok((0,false)); }
        let seq=self.shared.wal.lock().unwrap().append(key,value)?;
        let old=shard.active.insert(key.to_vec(),Entry {seq,value:value.map(|v|v.to_vec())});
        let size=key.len()+value.map_or(0,|v|v.len())+48;
        let old_size=old.as_ref().map_or(0,|v|key.len()+v.value.as_ref().map_or(0,Vec::len)+48);
        shard.active_bytes=shard.active_bytes.saturating_add(size).saturating_sub(old_size);
        if shard.active_bytes>=self.shared.cfg.spill_bytes { self.rotate(idx,&mut shard)?; }
        Ok((seq,existed))
    }
    pub fn set(&self,key:&[u8],value:&[u8])->io::Result<u64> { Ok(self.write(key,Some(value),false)?.0) }
    pub fn delete(&self,key:&[u8])->io::Result<u64> { Ok(self.write(key,None,false)?.0) }
    pub fn delete_count(&self,key:&[u8])->io::Result<bool> { Ok(self.write(key,None,true)?.1) }
    pub fn get(&self,key:&[u8])->io::Result<Option<Vec<u8>>> {
        self.validate(key,None)?;
        self.healthy()?;
        let shard=self.shared.shards[self.index(key)].lock().unwrap();
        self.lookup_locked(&shard,key)
    }
    fn barrier(&self)->io::Result<()> {
        let (tx,rx)=mpsc::channel();
        self.jobs.send(Job::Barrier(tx)).map_err(|_|invalid("flush worker unavailable"))?;
        rx.recv().map_err(|_|invalid("flush barrier failed"))?;
        self.healthy()
    }
    pub fn checkpoint(&self)->io::Result<u64> {
        let fence;
        {
            let _gate=self.shared.gate.lock().unwrap();
            self.healthy()?;
            fence=self.shared.wal.lock().unwrap().seq;
            for idx in 0..self.shared.cfg.shards {
                let mut shard=self.shared.shards[idx].lock().unwrap();
                self.rotate(idx,&mut shard)?;
            }
        }
        self.barrier()?;
        let _gate=self.shared.gate.lock().unwrap();
        self.healthy()?;
        {
            let mut man=self.shared.manifest.lock().unwrap();
            man.fence=man.fence.max(fence);
            for mark in &mut man.marks { *mark=(*mark).max(fence); }
            man.save(&self.shared.dir)?;
        }
        self.shared.wal.lock().unwrap().prune(fence,&self.shared.cfg)?;
        Ok(fence)
    }
    pub fn compact(&self)->io::Result<usize> {
        self.checkpoint()?;
        let _gate=self.shared.gate.lock().unwrap();
        self.barrier()?;
        let mut count=0;
        for idx in 0..self.shared.cfg.shards {
            let mut shard=self.shared.shards[idx].lock().unwrap();
            if shard.segments.len()<2 { continue; }
            let old=shard.segments.clone();
            let mut merged:BTreeMap<Vec<u8>,Entry>=BTreeMap::new();
            for s in &old {
                for key in s.index.keys() {
                    if let Some(e)=s.get(key,&self.shared.cfg)? {
                        if merged.get(key).is_none_or(|prev|e.seq>prev.seq) { merged.insert(key.clone(),e); }
                    }
                }
            }
            merged.retain(|_,e|e.value.is_some());
            let new=if merged.is_empty(){None}else{Some(Segment::write(&self.shared.dir,idx,&merged,&self.shared.cfg)?)};
            {
                let mut man=self.shared.manifest.lock().unwrap();
                man.files[idx]=new.as_ref().map_or_else(Vec::new,|s|vec![s.name.clone()]);
                man.save(&self.shared.dir)?;
            }
            shard.segments=new.into_iter().collect();
            for s in old { fs::remove_file(s.path)?; }
            sync_dir(&self.shared.dir)?;
            count+=1;
        }
        Ok(count)
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _=self.jobs.send(Job::Exit);
        if let Ok(mut handle)=self.worker.lock() {
            if let Some(t)=handle.take() { let _=t.join(); }
        }
        if let Ok(wal)=self.shared.wal.lock() { let _=wal.f.sync_all(); }
    }
}
