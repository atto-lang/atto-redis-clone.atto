//! Bounded RESP2 parser and loopback-by-default TCP server.
//! A small subset, not full Redis compatibility.
use crate::Engine;
use std::io::{self, BufReader, Read, Write};
use std::net::{TcpListener,TcpStream};
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
use std::thread;
use std::time::Duration;
const MAX_ARGS: usize=16;
const MAX_LINE: usize=128;
const MAX_CONNECTIONS: usize=32;
// Limits bound parser allocations even when the storage engine allows larger values.
const MAX_NETWORK_BULK: usize=4*1024*1024;
const MAX_REQUEST_BYTES: usize=8*1024*1024;
fn bad(s:&str)->io::Error { io::Error::new(io::ErrorKind::InvalidData,s) }
fn line(r:&mut BufReader<TcpStream>)->io::Result<Option<Vec<u8>>> {
    let mut v=Vec::new();
    for _ in 0..MAX_LINE {
        let mut b=[0u8;1];
        if r.read(&mut b)?==0 {return if v.is_empty(){Ok(None)}else{Err(bad("truncated header"))};}
        v.push(b[0]);
        if b[0]==b'\n' {
            if v.len()<2 || v[v.len()-2]!=b'\r'{return Err(bad("expected CRLF"));}
            v.truncate(v.len()-2);return Ok(Some(v));
        }
    }
    Err(bad("header too long"))
}
fn count(v:&[u8])->io::Result<usize>{
    if v.is_empty() || !v.iter().all(|c|c.is_ascii_digit()){return Err(bad("invalid length"));}
    std::str::from_utf8(v).map_err(|_|bad("bad length"))?
        .parse().map_err(|_|bad("length overflow"))
}
fn command(r:&mut BufReader<TcpStream>,max_key:usize,max_value:usize)->io::Result<Option<Vec<Vec<u8>>>>{
    let Some(h)=line(r)? else{return Ok(None)};
    if h.first()!=Some(&b'*'){return Err(bad("expected array"));}
    let n=count(&h[1..])?;
    if !(1..=MAX_ARGS).contains(&n){return Err(bad("argument count"));}
    let mut args:Vec<Vec<u8>>=Vec::with_capacity(n);
    let mut total=0usize;
    for index in 0..n{
        let Some(b)=line(r)? else{return Err(bad("truncated bulk"))};
        if b.first()!=Some(&b'$'){return Err(bad("expected bulk"));}
        let size=count(&b[1..])?;
        let cap=if index==0 {64} else if index==2 && args[0].eq_ignore_ascii_case(b"SET")
            || index==1 && (args[0].eq_ignore_ascii_case(b"PING") || args[0].eq_ignore_ascii_case(b"ECHO")) {
            max_value.min(MAX_NETWORK_BULK)
        } else {max_key.min(MAX_NETWORK_BULK)};
        total=total.checked_add(size).ok_or_else(||bad("request length overflow"))?;
        if size>cap || total>MAX_REQUEST_BYTES{return Err(bad("oversized request"));}
        let mut v=vec![0;size];r.read_exact(&mut v)?;
        let mut crlf=[0;2];r.read_exact(&mut crlf)?;
        if crlf!=*b"\r\n"{return Err(bad("bad trailer"));}
        args.push(v);
    }
    Ok(Some(args))
}
fn bulk(v:Option<&[u8]>)->Vec<u8>{
    let Some(v)=v else{return b"$-1\r\n".to_vec()};
    let mut out=format!("{}{}\r\n","$",v.len()).into_bytes();
    out.extend_from_slice(v);out.extend_from_slice(b"\r\n");out
}
pub fn serve_connection(stream:TcpStream,engine:Arc<Engine>)->io::Result<()>{
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader=BufReader::new(stream.try_clone()?);
    let mut writer=stream;
    let cfg=engine.config();
    // Enforce argument-position-specific limits before allocating a bulk body.
    loop{
        let args=match command(&mut reader,cfg.max_key_bytes,cfg.max_value_bytes){
            Ok(Some(x))=>x,Ok(None)=>return Ok(()),
            Err(_)=>{writer.write_all(b"-ERR invalid RESP request\r\n")?;return Ok(());}
        };
        let op=args[0].iter().map(u8::to_ascii_uppercase).collect::<Vec<_>>();
        let response=match (op.as_slice(),args.len()){
            (b"PING",1)=>b"+PONG\r\n".to_vec(),
            (b"PING",2)|(b"ECHO",2)=>bulk(Some(&args[1])),
            (b"SET",3)=>match engine.set(&args[1],&args[2]){
                Ok(_)=>b"+OK\r\n".to_vec(),
                Err(_)=>b"-ERR storage operation failed\r\n".to_vec(),
            },
            (b"GET",2)=>match engine.get(&args[1]){
                Ok(v)=>bulk(v.as_deref()),
                Err(_)=>b"-ERR storage operation failed\r\n".to_vec(),
            },
            (b"DEL",2)=>match engine.delete_count(&args[1]){
                Ok(true)=>b":1\r\n".to_vec(),
                Ok(false)=>b":0\r\n".to_vec(),
                Err(_)=>b"-ERR storage operation failed\r\n".to_vec(),
            },
            (b"QUIT",1)=>{writer.write_all(b"+OK\r\n")?;return Ok(());}
            _=>b"-ERR unsupported command or argument count\r\n".to_vec(),
        };
        writer.write_all(&response)?;
    }
}
pub fn serve(addr:&str,engine:Arc<Engine>)->io::Result<()>{
    let listener=TcpListener::bind(addr)?;
    let active=Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming(){
        let mut stream=incoming?;
        if active.fetch_add(1,Ordering::AcqRel)>=MAX_CONNECTIONS{
            active.fetch_sub(1,Ordering::AcqRel);
            let _=stream.write_all(b"-ERR too many clients\r\n");continue;
        }
        let count=Arc::clone(&active);let db=Arc::clone(&engine);
        thread::spawn(move||{
            let _=serve_connection(stream,db);
            count.fetch_sub(1,Ordering::AcqRel);
        });
    }
    Ok(())
}
