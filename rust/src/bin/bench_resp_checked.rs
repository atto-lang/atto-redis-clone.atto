//! Checked benchmark: real Atto actor+JDK RESP bridge versus Rust durable reference.
//! This benchmark validates replies; the two configurations are NOT equivalent.
use std::{collections::HashMap, env, io::{self, BufRead, BufReader, Read, Write},
          net::TcpStream, time::{Duration, Instant}};
#[derive(Debug, PartialEq)]
enum Reply { Simple(Vec<u8>), Int(i64), Bulk(Option<Vec<u8>>) }
fn bad(s:&str)->io::Error { io::Error::new(io::ErrorKind::InvalidData,s) }
fn header<R:BufRead>(r:&mut R)->io::Result<Vec<u8>> {
    let mut v=Vec::new();
    let n=r.take(128).read_until(b'\n', &mut v)?;
    if n<2 || !v.ends_with(b"\r\n") { return Err(bad("invalid RESP line")); }
    v.truncate(n-2); Ok(v)
}
fn recv<R:BufRead>(r:&mut R)->io::Result<Reply> {
    let h=header(r)?;
    if h.is_empty(){return Err(bad("empty RESP"));}
    match h[0] {
        b'+' => Ok(Reply::Simple(h[1..].to_vec())),
        b':' => Ok(Reply::Int(std::str::from_utf8(&h[1..])
            .map_err(|_|bad("integer UTF8"))?.parse().map_err(|_|bad("integer parse"))?)),
        b'$' => {
            let n:i64=std::str::from_utf8(&h[1..])
                .map_err(|_|bad("bulk UTF8"))?.parse().map_err(|_|bad("bulk parse"))?;
            if n == -1 {return Ok(Reply::Bulk(None));}
            if !(0..=1048576).contains(&n) {return Err(bad("bulk exceeds cap"));}
            let mut v=vec![0u8;n as usize+2];r.read_exact(&mut v)?;
            if !v.ends_with(b"\r\n"){return Err(bad("bad bulk CRLF"));}
            v.truncate(n as usize);Ok(Reply::Bulk(Some(v)))
        },
        b'-' => Err(bad("server returned error")),
        _ => Err(bad("unsupported reply")),
    }
}
fn cmd(items:&[&[u8]])->Vec<u8>{
    let mut b=format!("*{}\r\n",items.len()).into_bytes();
    for x in items {b.extend_from_slice(format!("{}{}\r\n",'$',x.len()).as_bytes());
        b.extend_from_slice(x);b.extend_from_slice(b"\r\n");} b
}
fn op(i:usize, trial:usize, state:&mut HashMap<Vec<u8>,Vec<u8>>)->(Vec<u8>,Reply){
    let block=i/10;let offset=i%10;
    let slot=(block.wrapping_mul(13)+match offset{5=>0,6=>1,7=>2,8=>9,9=>0,x=>x})%1024;
    let key=format!("bench-t{trial}-k{slot}").into_bytes();
    match offset {
        0..=4 => {let val=vec![(i%251) as u8;64];state.insert(key.clone(),val.clone());
                   (cmd(&[b"SET",&key,&val]),Reply::Simple(b"OK".to_vec()))}
        5..=8 => (cmd(&[b"GET",&key]),Reply::Bulk(state.get(&key).cloned())),
        _ => {let existed=state.remove(&key).is_some();
              (cmd(&[b"DEL",&key]),Reply::Int(if existed{1}else{0}))}
    }
}
fn run(addr:&str,label:&str,trial:usize,n:usize,warmup:usize)->io::Result<f64>{
    let socket=TcpStream::connect(addr)?;socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(20)))?;
    socket.set_write_timeout(Some(Duration::from_secs(20)))?;
    let mut reader=BufReader::new(socket.try_clone()?);let mut writer=socket;
    let mut state=HashMap::new();let mut latency=Vec::with_capacity(n);
    let mut started=None;
    for i in 0..warmup+n {
        if i==warmup{started=Some(Instant::now());}
        let (bytes,expected)=op(i,trial,&mut state);
        let before=Instant::now();writer.write_all(&bytes)?;
        let actual=recv(&mut reader)?;
        if actual!=expected {return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("reply mismatch label={label} trial={trial} op={i}: got={actual:?} want={expected:?}")));}
        if i>=warmup{latency.push(before.elapsed().as_micros());}
    }
    let secs=started.expect("requests positive").elapsed().as_secs_f64();
    latency.sort_unstable();let p=|v:usize|latency[(latency.len()-1)*v/100];
    let rate=n as f64/secs;
    println!("BENCH label={label} trial={} checks={} measured={} ops_per_sec={rate:.1} p50_us={} p95_us={} p99_us={}",
        trial+1,n+warmup,n,p(50),p(95),p(99)); Ok(rate)
}
fn series(addr:&str,label:&str,trials:usize,n:usize,w:usize)->io::Result<f64>{
    let mut rates=Vec::new();
    for trial in 0..trials{rates.push(run(addr,label,trial,n,w)?);}
    rates.sort_by(f64::total_cmp);let med=rates[trials/2];
    println!("BENCH_MEDIAN label={label} trials={trials} ops_per_sec={med:.1} correctness=PASS");
    Ok(med)
}
fn main()->io::Result<()>{
    let args:Vec<String>=env::args().collect();let (mut atto,mut rust)=(None,None);
    let (mut n,mut w,mut trials)=(500usize,50usize,3usize);
    let mut i=1;while i<args.len(){
        if i+1>=args.len(){return Err(bad("missing option value"));}
        match args[i].as_str(){
            "--atto"=>atto=Some(args[i+1].clone()),
            "--rust"=>rust=Some(args[i+1].clone()),
            "--requests"=>n=args[i+1].parse().map_err(|_|bad("requests"))?,
            "--warmup"=>w=args[i+1].parse().map_err(|_|bad("warmup"))?,
            "--trials"=>trials=args[i+1].parse().map_err(|_|bad("trials"))?,
            _=>return Err(bad("unknown option")),
        }i+=2;
    }
    if atto.is_none()&&rust.is_none(){return Err(bad("no endpoint"));}
    if n==0||n>1000000||w>100000||trials==0||trials>20{return Err(bad("invalid sizes"));}
    println!("BENCH_CONTEXT=50percent_SET_40percent_GET_10percent_DEL 64byte_values 1024key_space verified_replies warmup={w} requests={n} trials={trials}");
    println!("BENCH_ATTO_SCOPE=Atto_guest_actor_plus_JDK21_TCP_bridge_no_WAL");
    let r=if let Some(a)=rust{Some(series(&a,"rust_fsync_always",trials,n,w)?)}else{None};
    let a=if let Some(a)=atto{Some(series(&a,"atto_actor_bridge_ephemeral",trials,n,w)?)}else{None};
    if let (Some(r),Some(a))=(r,a){println!("BENCH_RATIO atto_bridge_over_rust={:.4} comparable_durability=false",a/r);}
    Ok(())
}
#[cfg(test)]
mod tests{
    use super::*;use std::io::Cursor;
    #[test] fn decode_nil_empty_binary_and_count(){
        assert_eq!(recv(&mut Cursor::new(b"$-1\r\n")).unwrap(),Reply::Bulk(None));
        assert_eq!(recv(&mut Cursor::new(b"$0\r\n\r\n")).unwrap(),Reply::Bulk(Some(vec![])));
        assert_eq!(recv(&mut Cursor::new(b"$2\r\n\x00\xff\r\n")).unwrap(),Reply::Bulk(Some(vec![0,255])));
        assert_eq!(recv(&mut Cursor::new(b":1\r\n")).unwrap(),Reply::Int(1));
    }
    #[test] fn reject_invalid_and_unbounded_replies(){
        for x in [b"-ERR\r\n".as_slice(),b"$4\r\nab".as_slice(),
            b"$2\r\nxy!!".as_slice(),b"$1048577\r\n".as_slice(),
            b":oops\r\n".as_slice(),b"+bad".as_slice()]{
            assert!(recv(&mut Cursor::new(x)).is_err());
        }
    }
    #[test] fn oracle_checks_deleted_and_missing(){
        let mut h=HashMap::new();
        assert!(matches!(op(5,0,&mut h).1,Reply::Bulk(None)));
        op(0,0,&mut h);
        assert!(matches!(op(5,0,&mut h).1,Reply::Bulk(Some(_))));
        assert_eq!(op(9,0,&mut h).1,Reply::Int(1));
        assert_eq!(op(9,0,&mut h).1,Reply::Int(0));
        assert_eq!(op(5,0,&mut h).1,Reply::Bulk(None));
    }
    #[test] fn binary_frame(){assert_eq!(cmd(&[b"SET",b"\0",b"\xff"]),
        b"*3\r\n$3\r\nSET\r\n$1\r\n\0\r\n$1\r\n\xff\r\n");}
}
