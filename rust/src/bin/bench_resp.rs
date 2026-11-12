//! Oracle-verified RESP2 benchmark for Rust reference and real Atto guest.
//! Blocking TCP is test-client-only: never reuse on actor/NIO/CONTROL paths.
use std::{collections::HashMap,env,io::{self,Read,Write},net::TcpStream,
          thread,time::{Duration,Instant}};

fn err(s:&str)->io::Error{io::Error::new(io::ErrorKind::InvalidData,s)}

#[derive(Debug,Clone,Eq,PartialEq)]
enum Reply { Simple(Vec<u8>), Integer(i64), Bulk(Option<Vec<u8>>) }
fn command(args:&[&[u8]])->Vec<u8>{
    let mut v=format!("*{}\r\n",args.len()).into_bytes();
    for a in args {
        v.extend_from_slice(format!("{}{}\r\n",'$',a.len()).as_bytes());
        v.extend_from_slice(a);v.extend_from_slice(b"\r\n");
    } v
}
fn line<R:Read>(r:&mut R)->io::Result<Vec<u8>>{
    let mut v=Vec::with_capacity(32);
    for _ in 0..128 {
        let mut b=[0];r.read_exact(&mut b)?;v.push(b[0]);
        if b[0]==b'\n' {
            if v.len()<2||v[v.len()-2]!=b'\r'{return Err(err("RESP line not CRLF"))}
            v.truncate(v.len()-2);return Ok(v)
        }
    } Err(err("RESP line exceeds bound"))
}
fn reply<R:Read>(r:&mut R)->io::Result<Reply>{
    let l=line(r)?;
    let (&prefix,body)=l.split_first().ok_or_else(||err("empty reply"))?;
    match prefix {
        b'+'=>Ok(Reply::Simple(body.to_vec())),
        b':'=>Ok(Reply::Integer(std::str::from_utf8(body)
            .map_err(|_|err("integer encoding"))?.parse()
            .map_err(|_|err("integer syntax"))?)),
        b'$'=>{
            let n:i64=std::str::from_utf8(body).map_err(|_|err("bulk encoding"))?
                .parse().map_err(|_|err("bulk length syntax"))?;
            if n == -1{return Ok(Reply::Bulk(None))}
            if !(0..=1_048_576).contains(&n){return Err(err("bulk length bound"))}
            let mut data=vec![0;(n as usize)+2];r.read_exact(&mut data)?;
            if !data.ends_with(b"\r\n"){return Err(err("bulk terminator"))}
            data.truncate(n as usize);Ok(Reply::Bulk(Some(data)))
        }
        b'-'=>Err(io::Error::other(
            format!("server error: {}",String::from_utf8_lossy(body)))),
        _=>Err(err("unsupported RESP reply")),
    }
}
#[derive(Clone)]
struct Config {
    rust:Option<String>,atto:Option<String>,requests:usize,warmup:usize,
    clients:usize,pipeline:usize,keys:usize,value_bytes:usize,trials:usize,seed:u64,
}
fn config()->io::Result<Config>{
    let mut c=Config{rust:None,atto:None,requests:10_000,warmup:1_000,
        clients:1,pipeline:1,keys:1024,value_bytes:64,trials:5,seed:0xA7705EED};
    let args:Vec<String>=env::args().collect();let mut i=1;
    while i<args.len(){
        let v=args.get(i+1).ok_or_else(||err("missing option argument"))?;
        let n=||v.parse::<usize>().map_err(|_|err("invalid numeric option"));
        match args[i].as_str(){
            "--rust"=>c.rust=Some(v.clone()),
            "--atto"=>c.atto=Some(v.clone()),
            "--requests"=>c.requests=n()?,
            "--warmup"=>c.warmup=n()?,
            "--clients"=>c.clients=n()?,
            "--pipeline"=>c.pipeline=n()?,
            "--keys"=>c.keys=n()?,
            "--value-bytes"=>c.value_bytes=n()?,
            "--trials"=>c.trials=n()?,
            "--seed"=>c.seed=v.parse().map_err(|_|err("seed"))?,
            _=>return Err(err("unknown option")),
        }i+=2;
    }
    if !(1..=2_000_000).contains(&c.requests)||!(1..=1_000).contains(&c.clients)
        ||!(1..=256).contains(&c.pipeline)||!(1..=1_000_000).contains(&c.keys)
        ||c.value_bytes>65_536||c.warmup>1_000_000
        ||!(1..=25).contains(&c.trials)||c.requests<c.clients {
        return Err(err("benchmark resource bound"));
    }
    if c.rust.is_none()&&c.atto.is_none(){return Err(err("missing endpoint"))}
    Ok(c)
}
fn operation(i:usize,client:usize,trial:usize,c:&Config,
    oracle:&mut HashMap<Vec<u8>,Vec<u8>>)->(Vec<u8>,Reply) {
    // Independent per-client keys make multi-client response oracles deterministic.
    let key=format!("atto-bench/{trial}/{client}/{}",(i/10)%c.keys).into_bytes();
    match i%10 {
        0..=4=>{
            let val:Vec<u8>=(0..c.value_bytes).map(|j|
                ((i as u64*17+j as u64*131+c.seed)&255) as u8).collect();
            oracle.insert(key.clone(),val.clone());
            (command(&[b"SET",&key,&val]),Reply::Simple(b"OK".to_vec()))
        }
        5..=8=>(command(&[b"GET",&key]),Reply::Bulk(oracle.get(&key).cloned())),
        _=>{
            let found=oracle.remove(&key).is_some();
            (command(&[b"DEL",&key]),Reply::Integer(if found{1}else{0}))
        }
    }
}
struct PhaseSpec {count:usize,offset:usize,id:usize,trial:usize,measured:bool}
fn phase(socket:&mut TcpStream,spec:PhaseSpec,c:&Config,
    oracle:&mut HashMap<Vec<u8>,Vec<u8>>,lat:&mut Vec<u64>)->io::Result<()>{
    let PhaseSpec{count,offset,id,trial,measured}=spec;
    let mut done=0;
    while done<count {
        let n=c.pipeline.min(count-done);let mut requests=Vec::new();
        let mut expected=Vec::with_capacity(n);
        for j in 0..n {
            let (request,wanted)=operation(offset+done+j,id,trial,c,oracle);
            if requests.len()+request.len()>32*1024*1024 {
                return Err(err("pipeline burst exceeds 32 MiB"));
            }
            requests.extend_from_slice(&request);expected.push(wanted);
        }
        let sent=Instant::now();socket.write_all(&requests)?;
        for (j,wanted) in expected.iter().enumerate(){
            let actual=reply(socket)?;
            if actual!=*wanted {
                return Err(io::Error::new(io::ErrorKind::InvalidData,format!(
                    "RESP_ORACLE_FAILURE client={id} trial={trial} operation={} \
                     expected={:?} actual={:?}",offset+done+j,
                     std::mem::discriminant(wanted),std::mem::discriminant(&actual))));
            }
            if measured {lat.push(sent.elapsed().as_nanos().min(u64::MAX as u128) as u64)}
        }done+=n;
    }Ok(())
}
struct Worker {start:Instant,end:Instant,count:usize,lat:Vec<u64>}
fn client(addr:String,id:usize,trial:usize,c:Config)
    ->io::Result<Worker> {
    // Guest compilation/startup may outlast benchmark process launch.
    // Retry only connection-refused before the untimed warmup begins.
    let mut attempt=0;
    let mut s=loop {
        match TcpStream::connect(&addr) {
            Ok(socket)=>break socket,
            Err(failure) if failure.kind()==io::ErrorKind::ConnectionRefused
                             && attempt<200 => {
                attempt+=1;
                thread::sleep(Duration::from_millis(20));
            },
            Err(failure)=>return Err(failure),
        }
    };
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    s.set_write_timeout(Some(Duration::from_secs(20)))?;
    let mut oracle=HashMap::new();
    let n=c.requests/c.clients+usize::from(id<c.requests%c.clients);
    let warm=c.warmup/c.clients+usize::from(id<c.warmup%c.clients);
    let mut lat=Vec::with_capacity(n);
    phase(&mut s,PhaseSpec{count:warm,offset:0,id,trial,measured:false},
          &c,&mut oracle,&mut lat)?;
    let start=Instant::now();
    phase(&mut s,PhaseSpec{count:n,offset:warm,id,trial,measured:true},
          &c,&mut oracle,&mut lat)?;
    let end=Instant::now();
    // Cleanup (not timed), verifying all remaining values are deleted.
    let keys:Vec<Vec<u8>>=oracle.keys().cloned().collect();
    for key in keys {
        s.write_all(&command(&[b"DEL",&key]))?;
        if reply(&mut s)?!=Reply::Integer(1) {
            return Err(err("RESP_ORACLE_FAILURE cleanup"));
        }
    }
    Ok(Worker{start,end,count:n,lat})
}
fn pct(sorted:&[u64],p:usize)->u64{sorted[(sorted.len()-1)*p/100]/1000}
fn run(addr:&str,c:&Config,trial:usize)->io::Result<(f64,u64,u64,u64,f64)>{
    let mut workers=Vec::new();
    for id in 0..c.clients {
        let a=addr.to_owned();let config=c.clone();
        workers.push(thread::spawn(move||client(a,id,trial,config)));
    }
    let mut results=Vec::new();
    for w in workers {
        results.push(w.join().map_err(|_|err("benchmark worker panicked"))??);
    }
    let start=results.iter().map(|x|x.start).min().ok_or_else(||err("no clients"))?;
    let end=results.iter().map(|x|x.end).max().ok_or_else(||err("no clients"))?;
    let count:usize=results.iter().map(|x|x.count).sum();
    let rate=count as f64/end.duration_since(start).as_secs_f64().max(1e-9);
    let per:Vec<f64>=results.iter().map(|x|
        x.count as f64/x.end.duration_since(x.start).as_secs_f64().max(1e-9)).collect();
    let min=per.iter().copied().fold(f64::INFINITY,f64::min);
    let max=per.iter().copied().fold(0.,f64::max);
    let mut lat:Vec<u64>=results.into_iter().flat_map(|x|x.lat).collect();
    lat.sort_unstable();
    Ok((rate,pct(&lat,50),pct(&lat,95),pct(&lat,99),min/max))
}
fn main()->io::Result<()>{
    let c=config()?;let mut rust=Vec::new();let mut atto=Vec::new();
    println!("RESP_BENCH_CONFIG clients={} pipeline={} requests={} warmup={} \
              keys={} value_bytes={} trials={} seed={} oracle=strict \
              pipeline_latency=batch_submit_to_reply",c.clients,c.pipeline,
              c.requests,c.warmup,c.keys,c.value_bytes,c.trials,c.seed);
    for t in 0..c.trials {
        let order=if (t as u64+c.seed)%2==0{["rust","atto"]}else{["atto","rust"]};
        for target in order {
            let endpoint=if target=="atto"{&c.atto}else{&c.rust};
            if let Some(addr)=endpoint {
                let (rate,p50,p95,p99,fairness)=run(addr,&c,t)?;
                println!("RESP_BENCH_TRIAL target={} trial={} ops_per_sec={:.1} \
                          p50_us={} p95_us={} p99_us={} fairness_min_max={:.4} \
                          correctness=PASS",target,t+1,rate,p50,p95,p99,fairness);
                if target=="atto"{atto.push(rate)}else{rust.push(rate)}
            }
        }
    }
    for (name,v) in [("rust",&mut rust),("atto",&mut atto)] {
        if v.is_empty(){println!("{name}=NOT_EXECUTED");continue}
        v.sort_by(|a,b|a.total_cmp(b));
        println!("{name}_median_ops_per_sec={:.1} correctness=PASS",v[v.len()/2]);
    }
    if !rust.is_empty()&&!atto.is_empty(){
        println!("atto_over_rust_ratio={:.4}",atto[atto.len()/2]/rust[rust.len()/2]);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;use std::io::Cursor;
    #[test]fn binary_nil_empty_integer(){
        let mut r=Cursor::new(b"$5\r\n\x00\xff\r\nZ\r\n$0\r\n\r\n$-1\r\n:1\r\n+OK\r\n");
        assert_eq!(reply(&mut r).unwrap(),Reply::Bulk(Some(vec![0,255,13,10,b'Z'])));
        assert_eq!(reply(&mut r).unwrap(),Reply::Bulk(Some(vec![])));
        assert_eq!(reply(&mut r).unwrap(),Reply::Bulk(None));
        assert_eq!(reply(&mut r).unwrap(),Reply::Integer(1));
        assert_eq!(reply(&mut r).unwrap(),Reply::Simple(b"OK".to_vec()));
    }
    #[test]fn rejects_malformed(){
        for b in [b"$2\r\nokXX".as_slice(),b":x\r\n".as_slice(),
                  b"$1048577\r\n".as_slice(),b"+OK\n".as_slice(),b"*1\r\n".as_slice()]{
            assert!(reply(&mut Cursor::new(b)).is_err());
        }
    }
    #[test]fn checks_mutations_and_nil(){
        let c=Config{rust:None,atto:None,requests:10,warmup:0,
            clients:1,pipeline:1,keys:1,value_bytes:64,trials:1,seed:1};
        let mut m=HashMap::new();
        assert!(matches!(operation(0,0,0,&c,&mut m).1,Reply::Simple(_)));
        assert!(matches!(operation(5,0,0,&c,&mut m).1,Reply::Bulk(Some(_))));
        assert_eq!(operation(9,0,0,&c,&mut m).1,Reply::Integer(1));
        assert_eq!(operation(15,0,0,&c,&mut m).1,Reply::Bulk(None));
    }
}
