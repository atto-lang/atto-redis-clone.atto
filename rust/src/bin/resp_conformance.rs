//! Black-box RESP2 conformance client (std only, no Python).
//!
//! Usage: `resp_conformance 127.0.0.1:6380`
//!
//! Runs the same exact-byte checks against any server, so running it against
//! a real `redis-server` and against the native AttoLang server shows both
//! answer identically on the supported subset (PING, ECHO, GET, SET, DEL,
//! EXISTS, QUIT). Error replies are compared by their `-ERR` prefix only,
//! because Redis and AttoRedis word error messages differently.
//! Exit status: 0 = every check passed, 1 = a check failed, 2 = usage/IO.
use std::env;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

fn cmd(args: &[&[u8]]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        out.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        out.extend_from_slice(arg);
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn connect(addr: &str) -> io::Result<TcpStream> {
    let stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

fn read_exact_len(stream: &mut TcpStream, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

/// Read one complete RESP2 reply (simple, error, integer, bulk or nil).
fn read_reply(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte)?;
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            break;
        }
        if line.len() > 512 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "reply header too long"));
        }
    }
    if line[0] == b'$' {
        let text = std::str::from_utf8(&line[1..line.len() - 2])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bulk length not UTF-8"))?;
        let n: i64 = text
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bulk length not integer"))?;
        if n >= 0 {
            line.extend(read_exact_len(stream, n as usize + 2)?);
        }
    }
    Ok(line)
}

struct Checker {
    failures: usize,
    passed: usize,
}

impl Checker {
    fn expect(&mut self, name: &str, got: &[u8], want: &[u8]) {
        let ok = if want == b"-ERR" { got.starts_with(b"-ERR") } else { got == want };
        if ok {
            self.passed += 1;
        } else {
            self.failures += 1;
            eprintln!(
                "FAIL {name}: want {:?} got {:?}",
                String::from_utf8_lossy(want),
                String::from_utf8_lossy(got)
            );
        }
    }

    fn roundtrip(&mut self, s: &mut TcpStream, name: &str, args: &[&[u8]], want: &[u8]) -> io::Result<()> {
        s.write_all(&cmd(args))?;
        let got = read_reply(s)?;
        self.expect(name, &got, want);
        Ok(())
    }
}

fn run(addr: &str) -> io::Result<Checker> {
    let mut c = Checker { failures: 0, passed: 0 };
    let tag = format!("conf-{}", std::process::id());
    let k = |suffix: &str| format!("{tag}-{suffix}").into_bytes();
    let mut s = connect(addr)?;

    c.roundtrip(&mut s, "ping", &[b"PING"], b"+PONG\r\n")?;
    c.roundtrip(&mut s, "ping lowercase", &[b"ping"], b"+PONG\r\n")?;
    c.roundtrip(&mut s, "ping message", &[b"PING", b"hi"], b"$2\r\nhi\r\n")?;
    c.roundtrip(&mut s, "echo", &[b"ECHO", b"hello world"], b"$11\r\nhello world\r\n")?;
    c.roundtrip(&mut s, "echo empty", &[b"ECHO", b""], b"$0\r\n\r\n")?;
    c.roundtrip(&mut s, "get missing", &[b"GET", &k("missing")], b"$-1\r\n")?;
    c.roundtrip(&mut s, "set", &[b"SET", &k("a"), b"1"], b"+OK\r\n")?;
    c.roundtrip(&mut s, "get", &[b"GET", &k("a")], b"$1\r\n1\r\n")?;
    c.roundtrip(&mut s, "set overwrite", &[b"set", &k("a"), b"22"], b"+OK\r\n")?;
    c.roundtrip(&mut s, "get overwritten", &[b"get", &k("a")], b"$2\r\n22\r\n")?;
    let binary: &[u8] = b"a\r\nb\0c\xff";
    c.roundtrip(&mut s, "set binary", &[b"SET", &k("bin"), binary], b"+OK\r\n")?;
    c.roundtrip(&mut s, "get binary", &[b"GET", &k("bin")], b"$7\r\na\r\nb\0c\xff\r\n")?;
    c.roundtrip(&mut s, "set empty value", &[b"SET", &k("empty"), b""], b"+OK\r\n")?;
    c.roundtrip(&mut s, "get empty value", &[b"GET", &k("empty")], b"$0\r\n\r\n")?;
    c.roundtrip(&mut s, "exists", &[b"EXISTS", &k("a"), &k("missing"), &k("bin")], b":2\r\n")?;
    c.roundtrip(&mut s, "del", &[b"DEL", &k("a"), &k("missing"), &k("empty")], b":2\r\n")?;
    c.roundtrip(&mut s, "get deleted", &[b"GET", &k("a")], b"$-1\r\n")?;
    c.roundtrip(&mut s, "unknown command", &[b"NOSUCHCMD"], b"-ERR")?;
    c.roundtrip(&mut s, "get arity", &[b"GET"], b"-ERR")?;

    // A 64 KiB value crosses many TCP segments in both directions.
    let big = vec![b'x'; 65536];
    c.roundtrip(&mut s, "set 64KiB", &[b"SET", &k("big"), &big], b"+OK\r\n")?;
    let mut want_big = b"$65536\r\n".to_vec();
    want_big.extend_from_slice(&big);
    want_big.extend_from_slice(b"\r\n");
    c.roundtrip(&mut s, "get 64KiB", &[b"GET", &k("big")], &want_big)?;

    // Fragmented frame: split inside the length header and inside the body.
    let frame = cmd(&[b"SET", &k("frag"), b"fragmented"]);
    for chunk in [&frame[..3], &frame[3..9], &frame[9..]] {
        s.write_all(chunk)?;
        s.flush()?;
        std::thread::sleep(Duration::from_millis(20));
    }
    let got = read_reply(&mut s)?;
    c.expect("fragmented set", &got, b"+OK\r\n");
    c.roundtrip(&mut s, "get fragmented", &[b"GET", &k("frag")], b"$10\r\nfragmented\r\n")?;

    // Pipelining: many commands in one write, replies in order.
    let mut batch = Vec::new();
    for i in 0..100 {
        batch.extend(cmd(&[b"SET", &k(&format!("p{i}")), format!("v{i}").as_bytes()]));
        batch.extend(cmd(&[b"GET", &k(&format!("p{i}"))]));
    }
    s.write_all(&batch)?;
    for i in 0..100 {
        let set = read_reply(&mut s)?;
        c.expect(&format!("pipelined set {i}"), &set, b"+OK\r\n");
        let get = read_reply(&mut s)?;
        let v = format!("v{i}");
        let want = format!("${}\r\n{}\r\n", v.len(), v);
        c.expect(&format!("pipelined get {i}"), &get, want.as_bytes());
    }

    // Two clients share one key space; the second sees the first's write
    // while the first connection stays open and idle.
    let mut other = connect(addr)?;
    c.roundtrip(&mut s, "client A set", &[b"SET", &k("shared"), b"from-a"], b"+OK\r\n")?;
    c.roundtrip(&mut other, "client B get", &[b"GET", &k("shared")], b"$6\r\nfrom-a\r\n")?;
    c.roundtrip(&mut other, "client B set", &[b"SET", &k("shared"), b"from-b"], b"+OK\r\n")?;
    c.roundtrip(&mut s, "client A get", &[b"GET", &k("shared")], b"$6\r\nfrom-b\r\n")?;

    // Cleanup, then QUIT closes the connection after +OK.
    let mut cleanup: Vec<Vec<u8>> = vec![b"DEL".to_vec()];
    for key in ["bin", "big", "frag", "shared"] {
        cleanup.push(k(key));
    }
    for i in 0..100 {
        cleanup.push(k(&format!("p{i}")));
    }
    let refs: Vec<&[u8]> = cleanup.iter().map(|v| v.as_slice()).collect();
    c.roundtrip(&mut s, "cleanup", &refs, b":104\r\n")?;
    c.roundtrip(&mut other, "quit", &[b"QUIT"], b"+OK\r\n")?;
    let mut probe = [0u8; 1];
    let closed = matches!(other.read(&mut probe), Ok(0));
    c.expect("quit closes", if closed { b"closed" } else { b"open" }, b"closed");

    // Protocol error: a non-array frame gets an error reply.
    let mut bad = connect(addr)?;
    bad.write_all(b"*x\r\n")?;
    let got = read_reply(&mut bad)?;
    c.expect("protocol error", &got, b"-ERR");
    Ok(c)
}

fn main() -> ExitCode {
    let Some(addr) = env::args().nth(1) else {
        eprintln!("usage: resp_conformance HOST:PORT");
        return ExitCode::from(2);
    };
    match run(&addr) {
        Ok(c) => {
            println!("resp_conformance {addr}: {} passed, {} failed", c.passed, c.failures);
            if c.failures == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
        }
        Err(e) => {
            eprintln!("resp_conformance {addr}: io error: {e}");
            ExitCode::from(2)
        }
    }
}
