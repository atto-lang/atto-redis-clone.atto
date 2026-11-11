use atto_redis_clone::{Config, Engine};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
use std::thread;
use std::time::Duration;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("atto-redis-test-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn cfg() -> Config {
    Config { shards: 4, spill_bytes: 512, pending_limit: 256, ..Config::default() }
}
#[test]
fn set_get_delete_restart_and_empty_binary_values() {
    let tmp = Temp::new();
    {
        let db = Engine::open(&tmp.0, cfg()).unwrap();
        db.set(b"a\0b", b"hello\0world").unwrap();
        assert_eq!(db.get(b"a\0b").unwrap(), Some(b"hello\0world".to_vec()));
        db.set(b"empty", b"").unwrap();
        assert_eq!(db.get(b"empty").unwrap(), Some(Vec::new()));
        assert!(db.delete_count(b"a\0b").unwrap());
        assert!(!db.delete_count(b"a\0b").unwrap());
        assert_eq!(db.get(b"a\0b").unwrap(), None);
    }
    let db = Engine::open(&tmp.0, cfg()).unwrap();
    assert_eq!(db.get(b"a\0b").unwrap(), None);
    assert_eq!(db.get(b"empty").unwrap(), Some(Vec::new()));
}
#[test]
fn checkpoint_spill_compaction_reopen() {
    let tmp = Temp::new();
    {
        let db = Engine::open(&tmp.0, cfg()).unwrap();
        for n in 0..48 { db.set(format!("key-{n:04}").as_bytes(), &[n as u8; 64]).unwrap(); }
        assert!(db.checkpoint().unwrap() > 0);
        for n in 0..24 { db.delete(format!("key-{n:04}").as_bytes()).unwrap(); }
        db.checkpoint().unwrap();
        db.compact().unwrap();
    }
    let db = Engine::open(&tmp.0, cfg()).unwrap();
    for n in 0..48 {
        let found = db.get(format!("key-{n:04}").as_bytes()).unwrap();
        assert_eq!(found, if n < 24 { None } else { Some(vec![n as u8;64]) });
    }
}
#[test]
fn torn_tail_is_truncated_but_complete_checksum_error_is_rejected() {
    let tmp = Temp::new();
    {
        let db = Engine::open(&tmp.0, cfg()).unwrap();
        db.set(b"persist", b"data").unwrap();
    }
    let wal = tmp.0.join("WAL");
    let original = fs::read(&wal).unwrap();
    OpenOptions::new().append(true).open(&wal).unwrap().write_all(b"AWL1\x00\x00").unwrap();
    {
        let db = Engine::open(&tmp.0, cfg()).unwrap();
        assert_eq!(db.get(b"persist").unwrap(), Some(b"data".to_vec()));
    }
    assert_eq!(fs::read(&wal).unwrap(), original);
    let mut corrupt = original;
    *corrupt.last_mut().unwrap() ^= 1;
    fs::write(&wal, corrupt).unwrap();
    assert!(Engine::open(&tmp.0, cfg()).is_err());
}
#[test]
fn a_second_process_cannot_open_same_store() {
    let tmp = Temp::new();
    let first = Engine::open(&tmp.0, cfg()).unwrap();
    assert!(Engine::open(&tmp.0, cfg()).is_err());
    drop(first);
    assert!(Engine::open(&tmp.0, cfg()).is_ok());
}
#[test]
fn concurrent_disjoint_keys_recover() {
    let tmp = Temp::new();
    let db = Arc::new(Engine::open(&tmp.0, cfg()).unwrap());
    let threads: Vec<_> = (0..6).map(|t| {
        let db = Arc::clone(&db);
        thread::spawn(move || {
            for i in 0..40 {
                let key = format!("{t}:{i}");
                db.set(key.as_bytes(), key.as_bytes()).unwrap();
                assert_eq!(db.get(key.as_bytes()).unwrap().as_deref(), Some(key.as_bytes()));
            }
        })
    }).collect();
    for t in threads { t.join().unwrap(); }
    db.checkpoint().unwrap();
    drop(db);
    let db = Engine::open(&tmp.0, cfg()).unwrap();
    for t in 0..6 {
        for i in 0..40 {
            let key = format!("{t}:{i}");
            assert_eq!(db.get(key.as_bytes()).unwrap().as_deref(), Some(key.as_bytes()));
        }
    }
}
#[test]
fn resp_pipeline_and_binary_values_roundtrip() {
    let tmp = Temp::new();
    let db = Arc::new(Engine::open(&tmp.0, cfg()).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server_db = Arc::clone(&db);
    let server = thread::spawn(move || {
        let (client, _) = listener.accept().unwrap();
        atto_redis_clone::server::serve_connection(client, server_db).unwrap();
    });
    let mut c = TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    c.write_all(b"*3\r\n$3\r\nSET\r\n$3\r\nk\0y\r\n$3\r\nv\0z\r\n*2\r\n$3\r\nGET\r\n$3\r\nk\0y\r\n*2\r\n$3\r\nDEL\r\n$3\r\nk\0y\r\n*1\r\n$4\r\nQUIT\r\n").unwrap();
    let mut out = Vec::new();
    c.read_to_end(&mut out).unwrap();
    assert_eq!(out, b"+OK\r\n$3\r\nv\0z\r\n:1\r\n+OK\r\n");
    server.join().unwrap();
}
#[test]
fn invalid_resp_rejected_before_allocation() {
    let tmp = Temp::new();
    let db = Arc::new(Engine::open(&tmp.0, cfg()).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        atto_redis_clone::server::serve_connection(socket, db).unwrap();
    });
    let mut sock = TcpStream::connect(addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    sock.write_all(b"*2\r\n$4\r\nECHO\r\n$4294967295\r\n").unwrap();
    let mut out = Vec::new();
    sock.read_to_end(&mut out).unwrap();
    assert_eq!(out, b"-ERR invalid RESP request\r\n");
    task.join().unwrap();
}
