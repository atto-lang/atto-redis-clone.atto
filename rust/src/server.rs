//! Bounded RESP2 parser and loopback-by-default TCP server.
//! A small subset, not full Redis compatibility.
use crate::Engine;
use std::io::{self, BufReader, Read, Write};
use std::net::{TcpListener,TcpStream};
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
use std::thread;
use std::time::Duration;
const MAX_ARGS: usize=16;
const MAX_BULK_BYTES: usize=4 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize=8 * 1024 * 1024;
const MAX_COMMAND_BYTES: usize=32;
const MAX_LINE: usize=128;
const MAX_CONNECTIONS: usize=64;
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
// Cumulative budget is checked before reading or allocating the next body.
fn charge_request_bytes(consumed:usize,next:usize)->io::Result<usize>{
    let amount=consumed.checked_add(next)
        .ok_or_else(||bad("request length overflow"))?;
    if amount>MAX_REQUEST_BYTES{return Err(bad("oversized request"));}
    Ok(amount)
}
fn command(r:&mut BufReader<TcpStream>,max_bulk:usize)->io::Result<Option<Vec<Vec<u8>>>>{
    let Some(h)=line(r)? else{return Ok(None)};
    if h.first()!=Some(&b'*'){return Err(bad("expected array"));}
    let n=count(&h[1..])?;
    if !(1..=MAX_ARGS).contains(&n){return Err(bad("argument count"));}
    let mut args=Vec::with_capacity(n);
    let mut request_bytes=h.len()+2; // array header CRLF
    for i in 0..n{
        let Some(b)=line(r)? else{return Err(bad("truncated bulk"))};
        if b.first()!=Some(&b'$'){return Err(bad("expected bulk"));}
        let size=count(&b[1..])?;
        // Reject before any allocation or read of the declared body.
        if i==0 && size>MAX_COMMAND_BYTES{return Err(bad("command name too long"));}
        if size>max_bulk || size>MAX_BULK_BYTES{return Err(bad("oversized bulk"));}
        let frame_bytes=b.len().checked_add(2)
            .and_then(|x|x.checked_add(size))
            .and_then(|x|x.checked_add(2))
            .ok_or_else(||bad("request length overflow"))?;
        request_bytes=charge_request_bytes(request_bytes,frame_bytes)?;
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
    let max=cfg.max_key_bytes.max(cfg.max_value_bytes);
    loop{
        let args=match command(&mut reader,max){
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
struct ConnectionPermit(Arc<AtomicUsize>);
impl Drop for ConnectionPermit {
    fn drop(&mut self) { self.0.fetch_sub(1,Ordering::AcqRel); }
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
        let permit=ConnectionPermit(Arc::clone(&active));
        let db=Arc::clone(&engine);
        // A handler panic must not leak a client slot. Failed spawn drops its
        // captured permit rather than panicking the listener.
        let _=thread::Builder::new().spawn(move||{
            let _permit=permit;
            let _=serve_connection(stream,db);
        });
    }
    Ok(())
}

#[cfg(test)]
mod protocol_bounds_tests {
    use super::*;
    use std::net::TcpListener;

    fn make_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (receiver, _) = listener.accept().unwrap();
        receiver.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        (sender, receiver)
    }

    #[test]
    fn rejects_oversized_bulk_before_reading_payload() {
        let (mut sender, receiver) = make_pair();
        sender.write_all(b"*2\r\n$4\r\nECHO\r\n$4194305\r\n").unwrap();
        let err = command(&mut BufReader::new(receiver), 64 * 1024 * 1024).unwrap_err();
        assert!(err.to_string().contains("oversized bulk"), "{err}");
    }

    #[test]
    fn rejects_oversized_command_name_before_reading_payload() {
        let (mut sender, receiver) = make_pair();
        sender.write_all(b"*1\r\n$33\r\n").unwrap();
        let err = command(&mut BufReader::new(receiver), 64 * 1024 * 1024).unwrap_err();
        assert!(err.to_string().contains("command name too long"), "{err}");
    }

    #[test]
    fn request_budget_accounts_for_aggregate_bulk_overhead() {
        let consumed=charge_request_bytes(0,MAX_BULK_BYTES+24).unwrap();
        let overflow=charge_request_bytes(consumed,MAX_BULK_BYTES+24).unwrap_err();
        assert_eq!(overflow.to_string(),"oversized request");
    }
}
