# Oracle-checked RESP2 benchmark gate (draft)

The executable benchmark is a Rust **test client** at rust/src/bin/bench_resp.rs.
Blocking I/O is allowed only here, never on Atto actor/NIO/CONTROL threads.
The server still uses asynchronous NIO2 and actor-owned Bytes.

## Commands

Build/test with Rust 1.85+:

    cargo test --manifest-path rust/Cargo.toml --bin bench_resp --offline
    cargo build --manifest-path rust/Cargo.toml --release --bin bench_resp --offline

Use separately started loopback Atto and official Redis endpoints:

    rust/target/release/bench_resp --atto 127.0.0.1:6380 --rust 127.0.0.1:6379 --requests 20000 --warmup 2000 --trials 7 --clients 1 --pipeline 1 --keys 1024 --value-bytes 64 --seed 42
    rust/target/release/bench_resp --rust 127.0.0.1:6379 --requests 20000 --warmup 2000 --trials 7 --clients 16 --pipeline 64 --keys 1024 --value-bytes 1024 --seed 42

**Do not claim multi-client Atto certification.** The current Atto guest
accepts one connection and consumes an uncorrelated shared actor-output
stream. Multi-client routing requires per-request correlation and bounded
socket sessions before concurrent clients can safely share one KV actor.

The driver performs 50/40/10 SET/GET/DEL with per-client key namespaces,
deterministic binary values, and exact per-response oracle comparisons:
SET OK, GET existing/empty/nil, DEL 0/1, and cleanup. Any mismatch,
malformed response, connection failure, or timeout fails closed.

Results include aggregate ops/sec, p50/p95/p99 latency, and minimum/maximum
per-client throughput fairness. **Pipelined latency is time from BATCH
submission to each reply**, not isolated service time. Per-client warmup is
not globally synchronized, so wide concurrent benchmarks can include
warmup/measurement overlap. Use externally synchronized repeated tests
for definitive performance claims.

Endpoints are exercised with alternating ABBA order and identical workload
parameters, with cleanup outside timing. Redis servers may accept fewer
clients than the driver permits; failed admission is not a successful
performance result.

## Promotion criteria

1. Exact compiler/guest/benchmark SHAs, compatible checksum-pinned optimizing
   GraalVM, no Truffle interpreter fallback, no skipped or failed tests.
2. Same dedicated machine and CPU allocations for official Redis and Atto;
   separate ephemeral and WAL/fsync durability comparisons.
3. Clients 1/10/100/1000; pipelines 1/16/64/256; values 64B/1KiB/64KiB;
   storage 1K/100K/1M keys, with explicit resource bounds.
4. CPU/RSS/allocations/GC, p50/p95/p99, fairness, randomized runs and
   sufficiently long warmups.
5. Real Atto cross-client visibility and linearizability, bounded send
   admission, cancellation, supervision, CONTROL priority, and slow-reader
   isolation. Disjoint-key throughput alone does NOT prove these properties.
6. Keep draft PRs unmerged until exact-head executable evidence is linked.
