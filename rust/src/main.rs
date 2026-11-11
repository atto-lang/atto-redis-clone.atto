use atto_redis_clone::{server, Config, Engine};
use std::{env, io, net::{IpAddr, SocketAddr}, sync::Arc, thread, time::Duration};
fn bad(m: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, m) }
fn main() -> io::Result<()> {
    let mut argv = env::args().skip(1);
    let mut path = "./var/atto-redis".to_owned();
    let mut host = "127.0.0.1".to_owned();
    let mut port: u16 = 6379;
    let mut cfg = Config::default();
    let mut checkpoint = 60u64;
    while let Some(flag) = argv.next() {
        let v = argv.next().ok_or_else(|| bad("option requires value"))?;
        match flag.as_str() {
            "--data-dir" => path = v,
            "--host" => host = v,
            "--port" => port = v.parse().map_err(|_| bad("invalid port"))?,
            "--shards" => cfg.shards = v.parse().map_err(|_| bad("invalid shards"))?,
            "--spill-mib" => {
                cfg.spill_bytes = v.parse::<usize>().map_err(|_| bad("invalid spill limit"))?
                    .checked_mul(1024 * 1024).ok_or_else(|| bad("spill overflow"))?;
            }
            "--fsync" => cfg.fsync_every_second = match v.as_str() {
                "always" => false, "everysec" => true,
                _ => return Err(bad("invalid fsync mode")),
            },
            "--checkpoint-seconds" => checkpoint = v.parse().map_err(|_| bad("invalid checkpoint interval"))?,
            _ => return Err(bad("unsupported option")),
        }
    }
    if checkpoint == 0 { return Err(bad("checkpoint interval must be positive")); }
    // Remote unauthenticated Redis access is unsafe. Fail closed until an
    // authenticated and encrypted listener is explicitly implemented.
    let ip: IpAddr = host.parse().map_err(|_| bad("host must be a numeric loopback IP"))?;
    if !ip.is_loopback() { return Err(bad("remote listener forbidden without auth/TLS; use loopback")); }
    let addr = SocketAddr::new(ip, port);
    let engine = Arc::new(Engine::open(path, cfg)?);
    let keeper = Arc::clone(&engine);
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(checkpoint));
        if let Err(e) = keeper.checkpoint() { eprintln!("checkpoint error: {e}"); }
    });
    server::serve(&addr.to_string(), engine)
}
