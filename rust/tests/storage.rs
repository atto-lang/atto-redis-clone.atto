use atto_redis_clone::{Config, Engine, server};
use std::{fs, io::{Read,Write}, net::{TcpListener,TcpStream}, path::PathBuf, sync::Arc,
          thread, time::{Duration, SystemTime, UNIX_EPOCH}};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let n=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let p=std::env::temp_dir().join(format!("atto-redis-rust-{}-{n}",std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
fn cfg()->Config {Config{shards:4,spill_bytes:128,pending_limit:64,..Config::default()}}
#[test]
fn binary_values_restart_and_tombstones() {
    let dir=Temp::new(); let db=Engine::open(&dir.0,cfg()).unwrap();
    assert_eq!(db.get(b"k").unwrap(),None);
    assert_eq!(db.set(b"k",b"\0\xff\r\n").unwrap(),1);
    assert_eq!(db.get(b"k").unwrap(),Some(b"\0\xff\r\n".to_vec()));
    assert!(db.delete_count(b"k").unwrap());
    assert!(!db.delete_count(b"k").unwrap());
    db.set(b"empty",b"").unwrap();
    db.checkpoint().unwrap();drop(db);
    let r=Engine::open(&dir.0,cfg()).unwrap();
    assert_eq!(r.get(b"k").unwrap(),None);
    assert_eq!(r.get(b"empty").unwrap(),Some(vec![]));
}
#[test]
fn overwritten_disk_value_and_compaction() {
    let dir=Temp::new();let db=Engine::open(&dir.0,cfg()).unwrap();
    db.set(b"a",b"old").unwrap();db.checkpoint().unwrap();
    db.set(b"a",b"new").unwrap();db.set(b"b",b"exists").unwrap();
    db.delete(b"b").unwrap();db.checkpoint().unwrap();
    assert_eq!(db.get(b"a").unwrap(),Some(b"new".to_vec()));
    assert_eq!(db.get(b"b").unwrap(),None);
    db.compact().unwrap();drop(db);
    let r=Engine::open(&dir.0,cfg()).unwrap();
    assert_eq!(r.get(b"a").unwrap(),Some(b"new".to_vec()));
    assert_eq!(r.get(b"b").unwrap(),None);
}
#[test]
fn concurrent_clients_and_sparse_shard_fence() {
    let dir=Temp::new();let db=Arc::new(Engine::open(&dir.0,cfg()).unwrap());
    let mut workers=Vec::new();
    for t in 0..4 {
        let db=Arc::clone(&db);
        workers.push(thread::spawn(move|| {
            for n in 0..30 {
                let key=format!("key-{t}-{n}");
                db.set(key.as_bytes(),b"value").unwrap();
            }
        }));
    }
    for w in workers{w.join().unwrap();}
    assert_eq!(db.checkpoint().unwrap(),120);
    drop(db);
    let r=Engine::open(&dir.0,cfg()).unwrap();
    for t in 0..4{for n in 0..30 {
        assert_eq!(r.get(format!("key-{t}-{n}").as_bytes()).unwrap(),Some(b"value".to_vec()));
    }}
}
#[test]
fn corrupted_wal_fails_closed() {
    let dir=Temp::new();let db=Engine::open(&dir.0,cfg()).unwrap();
    db.set(b"k",b"v").unwrap();drop(db);
    let path=dir.0.join("WAL");let mut b=fs::read(&path).unwrap();
    let len=b.len();b[len-1]^=1;fs::write(&path,b).unwrap();
    assert!(Engine::open(&dir.0,cfg()).is_err());
}
#[test]
fn missing_manifest_fails_closed() {
    let dir=Temp::new();let db=Engine::open(&dir.0,cfg()).unwrap();
    db.set(b"k",b"v").unwrap();db.checkpoint().unwrap();drop(db);
    fs::remove_file(dir.0.join("MANIFEST")).unwrap();
    assert!(Engine::open(&dir.0,cfg()).is_err());
}
#[test]
fn oversized_inputs_are_rejected() {
    let dir=Temp::new();
    let config=Config{max_key_bytes:3,max_value_bytes:3,..cfg()};
    let db=Engine::open(&dir.0,config).unwrap();
    assert!(db.set(b"",b"x").is_err());
    assert!(db.set(b"toolong",b"x").is_err());
    assert!(db.set(b"k",b"toolong").is_err());
}
fn cmd(parts:&[&[u8]])->Vec<u8> {
    let mut out=format!("*{}\r\n",parts.len()).into_bytes();
    for p in parts {
        out.extend_from_slice(format!("{}{}\r\n","$",p.len()).as_bytes());
        out.extend_from_slice(p);out.extend_from_slice(b"\r\n");
    }
    out
}
#[test]
fn resp2_tcp_binary_pipeline() {
    let dir=Temp::new();let db=Arc::new(Engine::open(&dir.0,cfg()).unwrap());
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let addr=listener.local_addr().unwrap();
    let handle=thread::spawn(move|| {
        let (client,_)=listener.accept().unwrap();
        server::serve_connection(client,db).unwrap();
    });
    let mut stream=TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut sent=Vec::new();
    sent.extend(cmd(&[b"PING"])); sent.extend(cmd(&[b"SET",b"k",b"\x00\xff"]));
    sent.extend(cmd(&[b"GET",b"k"]));sent.extend(cmd(&[b"DEL",b"k"]));
    sent.extend(cmd(&[b"GET",b"k"]));sent.extend(cmd(&[b"QUIT"]));
    stream.write_all(&sent).unwrap();
    let mut buf=Vec::new();
    stream.read_to_end(&mut buf).unwrap();
    assert_eq!(buf,b"+PONG\r\n+OK\r\n$2\r\n\x00\xff\r\n:1\r\n$-1\r\n+OK\r\n");
    handle.join().unwrap();
}
