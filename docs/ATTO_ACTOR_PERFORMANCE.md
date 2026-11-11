# Verified Atto actor RESP2 performance: diagnostic baseline

This is a measurement of **real AttoLang actor-owned Map operations** reached through
the Java 21 loopback RESP2/IPC bridge. It is not a benchmark of an Atto-native TCP
listener, Atto WAL, SST, JDK25 optimized Truffle, GraalVM Native Image, or LLVM.

## Executed first baseline — 2026-11-11

- Private same-runner receipt: https://github.com/voxletra/voxletra-e2e/actions/runs/43089227338
- Carrier exact-head: `917b6a27ae79dc80a7cb257b13aae9fb7012cb29`
- Benchmark source blob: `79c3724b9550a266facc25e6461bde0fe4152a42`
- Atto guest blob: `c777f0ca9de51febacbf91d350e329c53614b059`
- 3 trials, 50 warm-up + 200 measured operations each, sequential one-connection RESP2.
- Mix: 50% SET, 40% GET, 10% DEL; 64-byte binary values; 1,024-key *possible*
  working set (short trial does not populate all 1,024 keys).
- Every response validated against a client oracle; no proxy-stored KV values.
- Atto actor+bridge: **501.4 / 810.3 / 1200.1 ops/sec**, median **810.3**.
- Rust native WAL fsync=always: **6219.0 / 5267.3 / 5336.8 ops/sec**,
  median **5336.8**.
- Atto first-trial p50/p95/p99: 1769/3520/4882 microseconds.
- Atto third-trial p50/p95/p99: 590/2479/3546 microseconds.
- Diagnostic throughput ratio: 0.1518 (15.18%). **NOT apples-to-apples.**

The continuous improvement of Atto across these trials indicates short-sample
warmup/scheduling sensitivity. Do not use these measurements to claim a stable
language-speed ranking. Rust is doing durable WAL fsync before success; Atto
has no persistence and pays extra JDK process/IPC/hex encoding costs.

## Reproduce on already-built running endpoints

```bash
cargo test --manifest-path rust/Cargo.toml --offline --bin bench_resp_checked
cargo run --manifest-path rust/Cargo.toml --offline --release \
  --bin bench_resp_checked -- \
  --rust 127.0.0.1:6379 --atto 127.0.0.1:6380 \
  --requests 1000 --warmup 200 --trials 5
```

The benchmark fails on incorrect GET/DEL/SET replies, malformed protocol data,
timeouts or connection failures; it never treats a protocol error as a successful
operation. Each trial uses its own key namespace and prints p50/p95/p99 latency.
It prints a ratio only when both endpoints have executed.

### Security and accuracy gates

1. Keep both unauthenticated servers loopback-only and on the same CI runner.
2. Verify the Atto source and Java compiler snapshot by exact Git blob SHA.
3. Require positive executed JUnit counts, no skipped/failing tests and a real
   Atto actor TCP exchange, rather than only host mock bridge tests.
4. Report JDK/Truffle mode, host resource environment, shard count and fsync
   policy separately for each endpoint.
5. For a native performance comparison, implement Atto TCP plus equivalent WAL,
   fsync, sharding and concurrency. Then benchmark identical settings and workloads.
