//! Executed Rust reference recovery and protocol regression tests.
//! These are not AttoLang compiler/runtime tests.
use atto_redis_clone::{server, Config, Engine};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// macOS clocks are microsecond-grained, so parallel test threads can read
// the same timestamp; a per-process counter keeps every directory unique.
static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "atto-redis-recovery-{}-{n}-{seq}",
            std::process::id()
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cfg() -> Config {
    Config {
        shards: 4,
        spill_bytes: 256,
        pending_limit: 64,
        ..Config::default()
    }
}

#[test]
fn exclusive_store_lease_fences_second_engine_and_releases_on_drop() {
    let dir = Temp::new();
    let first = Engine::open(&dir.0, cfg()).unwrap();
    first.set(b"before", b"durable").unwrap();
    assert!(Engine::open(&dir.0, cfg()).is_err());
    drop(first);
    let second = Engine::open(&dir.0, cfg()).unwrap();
    assert_eq!(second.get(b"before").unwrap(), Some(b"durable".to_vec()));
}

#[test]
fn torn_wal_suffix_is_truncated_then_lsn_continues() {
    let dir = Temp::new();
    let db = Engine::open(&dir.0, cfg()).unwrap();
    assert_eq!(db.set(b"k", b"first").unwrap(), 1);
    drop(db);
    let path = dir.0.join("WAL");
    let good_len = fs::metadata(&path).unwrap().len();
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"AWL")
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), good_len + 3);
    let recovered = Engine::open(&dir.0, cfg()).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), good_len);
    assert_eq!(recovered.get(b"k").unwrap(), Some(b"first".to_vec()));
    assert_eq!(recovered.set(b"k", b"second").unwrap(), 2);
    drop(recovered);
    let final_db = Engine::open(&dir.0, cfg()).unwrap();
    assert_eq!(final_db.get(b"k").unwrap(), Some(b"second".to_vec()));
}

#[test]
fn complete_corrupt_wal_frame_is_not_silently_salvaged() {
    let dir = Temp::new();
    let db = Engine::open(&dir.0, cfg()).unwrap();
    db.set(b"k", b"durable").unwrap();
    drop(db);
    let path = dir.0.join("WAL");
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(Engine::open(&dir.0, cfg()).is_err());
}

#[test]
fn corrupted_manifest_rejects_recovery_instead_of_resetting_store() {
    let dir = Temp::new();
    let db = Engine::open(&dir.0, cfg()).unwrap();
    db.set(b"k", b"durable").unwrap();
    db.checkpoint().unwrap();
    drop(db);
    let path = dir.0.join("MANIFEST");
    let mut bytes = fs::read(&path).unwrap();
    bytes[8] ^= 1; // Stored CRC, not an incomplete trailing WAL frame.
    fs::write(&path, bytes).unwrap();
    assert!(Engine::open(&dir.0, cfg()).is_err());
}

#[test]
fn repeated_checkpoint_and_restart_preserves_lsn_and_tombstones() {
    let dir = Temp::new();
    let db = Engine::open(&dir.0, cfg()).unwrap();
    assert_eq!(db.set(b"keep", b"one").unwrap(), 1);
    assert_eq!(db.set(b"erase", b"hidden").unwrap(), 2);
    db.checkpoint().unwrap();
    assert_eq!(db.delete(b"erase").unwrap(), 3);
    db.checkpoint().unwrap();
    drop(db);
    let db = Engine::open(&dir.0, cfg()).unwrap();
    assert_eq!(db.get(b"erase").unwrap(), None);
    assert_eq!(db.get(b"keep").unwrap(), Some(b"one".to_vec()));
    assert_eq!(db.set(b"later", b"four").unwrap(), 4);
    db.checkpoint().unwrap();
    drop(db);
    let db = Engine::open(&dir.0, cfg()).unwrap();
    assert_eq!(db.get(b"erase").unwrap(), None);
    assert_eq!(db.get(b"later").unwrap(), Some(b"four".to_vec()));
}

#[test]
fn oversized_resp2_bulk_header_returns_error_without_applying_command() {
    let dir = Temp::new();
    let db = Arc::new(Engine::open(&dir.0, cfg()).unwrap());
    let check = Arc::clone(&db);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        server::serve_connection(client, db).unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    // Bulk value size is invalid and no bulk body is supplied; must reject
    // before allocating or waiting for the body.
    client.write_all(b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$4294967296\r\n").unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    assert_eq!(response, b"-ERR invalid RESP request\r\n");
    worker.join().unwrap();
    assert_eq!(check.get(b"k").unwrap(), None);
}

#[cfg(unix)]
#[test]
fn wal_lock_symlink_is_rejected_before_opening_target() {
    use std::os::unix::fs::symlink;
    let dir = Temp::new();
    let victim = dir.0.join("outside-lock-target");
    fs::write(&victim, b"untouched").unwrap();
    symlink(&victim, dir.0.join("WAL.lock")).unwrap();
    assert!(Engine::open(&dir.0, cfg()).is_err());
    assert_eq!(fs::read(&victim).unwrap(), b"untouched");
}
