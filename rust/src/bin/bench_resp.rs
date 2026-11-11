//! Same RESP2 workload for Rust and Atto. No extrapolated Atto numbers.
use std::{env,io::{self,Read,Write},net::TcpStream,time::{Duration,Instant}};
fn invalid(s:&str)->io::Error{io::Error::new(io::ErrorKind::InvalidInput,s)}
fn cmd(args:&[&[u8]])->Vec<u8>{
    let mut v=format!("*{}\r\n",args.len()).into_bytes();
    for x in args {v.extend_from_slice(format!("{}{}\r\n","$",x.len()).as_bytes());v.extend_from_slice(x);v.extend_from_slice(b"\r\n");}
    v
}
fn readline(s:&mut TcpStream)->io::Result<Vec<u8>>{
    let mut v=Vec::new();
    for _ in 0..128 {
        let mut b=[0;1];s.read_exact(&mut b)?;v.push(b[0]);
        if b[0]==b'\n' {return Ok(v);}
    }
    Err(invalid("line too long"))
}
fn recv(s:&mut TcpStream)->io::Result<()>{
    let hdr=readline(s)?;
    match hdr[0] {
        b'+'=>Ok(()),
        b':'=>Ok(()),
        b'$'=>{
            let text=std::str::from_utf8(&hdr[1..hdr.len()-2]).map_err(|_|invalid("RESP length"))?;
            let n=text.parse::<i64>().map_err(|_|invalid("RESP integer"))?;
            if n<0 {return Ok(());}
            let len=usize::try_from(n).map_err(|_|invalid("negative length"))?;
            if len>64*1024*1024{return Err(invalid("unbounded response"));}
            let mut bytes=vec![0;len+2];s.read_exact(&mut bytes)?;
            if !bytes.ends_with(b"\r\n"){return Err(invalid("bad RESP terminator"));}
            Ok(())
        }
        b'-'=>Err(invalid("storage returned error")),
        _=>Err(invalid("unknown RESP reply")),
    }
}
fn run(addr:&str,n:usize)->io::Result<f64>{
    let mut s=TcpStream::connect(addr)?;
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    s.set_write_timeout(Some(Duration::from_secs(20)))?;
    // Constant key/value cardinality and 50% SET, 40% GET, 10% DEL.
    let value=[b'x';64];
    let start=Instant::now();
    for i in 0..n {
        let key=format!("bench-{}",i%1024);
        let bytes=match i%10 {
            0=>cmd(&[b"DEL",key.as_bytes()]),
            1..=4=>cmd(&[b"GET",key.as_bytes()]),
            _=>cmd(&[b"SET",key.as_bytes(),&value]),
        };
        s.write_all(&bytes)?;recv(&mut s)?;
    }
    Ok(n as f64/start.elapsed().as_secs_f64())
}
fn main()->io::Result<()>{
    let args:Vec<String>=env::args().collect();
    let mut rust=None;let mut atto=None;let mut requests=10_000usize;
    let mut i=1;
    while i<args.len(){
        if i+1>=args.len(){return Err(invalid("missing option value"));}
        match args[i].as_str(){
            "--rust"=>rust=Some(args[i+1].clone()),
            "--atto"=>atto=Some(args[i+1].clone()),
            "--requests"=>requests=args[i+1].parse().map_err(|_|invalid("invalid requests"))?,
            _=>return Err(invalid("unknown option")),
        }i+=2;
    }
    if requests==0{return Err(invalid("empty workload"));}
    let rust_rate=if let Some(addr)=rust {
        let r=run(&addr,requests)?;
        println!("rust_ops_per_sec={r:.1}");Some(r)
    }else{println!("rust=NOT_EXECUTED");None};
    if let Some(addr)=atto {
        let a=run(&addr,requests)?;
        println!("atto_ops_per_sec={a:.1}");
        if let Some(r)=rust_rate{println!("atto_over_rust_ratio={:.4}",a/r);}
    }else{println!("atto=NOT_EXECUTED (no compiled native Atto RESP server endpoint)");}
    Ok(())
}
