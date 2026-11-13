# Native AttoLang RESP2 server: executed evidence, 2026-11-13

`src/server.atto` is a Redis-compatible server written in AttoLang. RESP2 framing, command dispatch and reply encoding are AttoLang code. The runtime supplies only the spec chapter 26 primitives: async TCP and move-only `Bytes`. No host RESP codec is used. The `read_resp2_*`/`write_resp2_*` runtime members are not called, and no JDK, Go or Rust bridge sits in front of it.

## Architecture

- The accept loop moves each accepted `NioConnection` into its own session task.
- One store task exclusively owns the key space (`Map<Bytes, Bytes>`), the same single-owner model as Redis. Sessions reach it by message passing; each request carries the session's reply `Channel`. There is no shared mutable memory.
- Pipelined commands from one read are executed in order, and their replies are coalesced into one write.
- Commands: `PING [msg]`, `ECHO`, `GET`, `SET`, `DEL k...`, `EXISTS k...`, `QUIT`. Limits: 1 MiB per bulk string, 1024 arguments, 2 MiB of buffered unparsed input per connection.
- Not implemented: persistence, expiry, `SET` options, other data types, AUTH/TLS. Do not expose it beyond loopback.

## Compiler

Java/GraalVM backend, `oreslang-source.java` main `ecd79fcf57`. The server depends on these merged PRs:

| PR | What it enables |
| --- | --- |
| #2894 | Async NIO TCP |
| #3042 | Affine `NioConnection` task transfer |
| #3043 | `Bytes` v1 and `Bytes`-keyed `Map` |
| #3044 | `Channel`/`Bytes`/`Map`/`Tuple` Send, so requests can carry reply channels |
| #3045 | `Bytes` across async calls, `list.size` binding |

Runtime: Labs OpenJDK `25.0.4.1+1-jvmci-25.3-b22` with the matching Graal 25.3.4.1 compiler jars, so the Truffle optimizing runtime is active (no "fallback runtime" warning). LLVM and .NET cannot compile this program yet; see the parity notes below.

## Conformance

`rust/src/bin/resp_conformance.rs` is a std-only Rust black-box client. It runs 231 exact-byte checks, covering:

- every supported command, case-insensitively;
- binary values containing CR/LF/NUL/0xFF, and the empty value;
- a 64 KiB value;
- a frame fragmented inside its header and body;
- 100 pipelined SET/GET pairs;
- two clients sharing one key space;
- QUIT closing the connection;
- arity, unknown-command and protocol errors.

```sh
cargo build --release --offline --manifest-path rust/Cargo.toml --bin resp_conformance
rust/target/release/resp_conformance 127.0.0.1:6390   # redis-server 8.x: 231 passed, 0 failed
rust/target/release/resp_conformance 127.0.0.1:6380   # AttoRedis:        231 passed, 0 failed
```

## Throughput (diagnostic, not yet an acceptance run)

`redis-benchmark -t set,get -c 50 [-P 16] -q`: Apple Silicon laptop, loopback, both servers on the same machine, real Redis with `--save '' --appendonly no`. Median of 3 trials:

| Workload | redis-server | AttoRedis (Java JIT) | Ratio |
| --- | ---: | ---: | ---: |
| SET, 50 clients | 174,064 rps | 19,750 rps | 11.3% |
| GET, 50 clients | 182,983 rps | 15,282 rps | 8.4% |
| SET, 50 clients, pipeline 16 | 1,600,000 rps | 28,463 rps | 1.8% |
| GET, 50 clients, pipeline 16 | 2,000,000 rps | 19,023 rps | 0.95% |

**The 70% target is not met.** The main cause is structural: the Java backend runs guests through one tree-walking evaluator (`OresEvalRootNode`, about 12k lines, no specialised Truffle nodes). Graal therefore JIT-compiles the interpreter rather than partially evaluating guest code. Secondary costs are visible in the AttoLang code itself: two channel hops per command and fresh `Bytes` allocations for every reply.

Earlier README figures (62%–102% "Atto vs Rust") measured a JDK bridge whose RESP parsing ran in Java, against a Rust server with fsync enabled. They are not comparable with this table and should not be read as AttoLang performance.

## Parity gaps for this program

- **LLVM main:** no `while`/`loop` beyond the counted `for (let int ...)` form, no `List`/`string`/`Bytes`, no `async`/`await`, no actor messaging. Blocks are capped at 16 statements.
- **.NET main:** compiles one checked-i64 function only.
