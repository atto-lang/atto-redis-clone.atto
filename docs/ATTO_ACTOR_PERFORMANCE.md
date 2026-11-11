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

## Extended verified benchmark — five 1,000-operation trials

The five-trial sample succeeded on the second CI attempt, after a transient
Maven Central dependency-resolution failure was retried **without changing
the source**: https://github.com/voxletra/voxletra-e2e/actions/runs/43089527434

- Carrier exact head: `1d40e8c392f30f9faf8c1550d2ce9e9c74640898`
- GitHub job ID (successful attempt 2): `130763001022`
- Actual Atto JUnit: 4/4; real Atto actor RESP2 TCP exchange: PASS.
- Java RESP bridge bounds: 17 checks; process/contract: 29 checks.
- Rust checked benchmark unit tests: 4/4 PASS.
- Five trials of 200 warm-up + 1,000 measured operations; all server replies
  checked against the client-side oracle.
- Atto actor + JDK21 bridge: **916.6 / 1821.6 / 2160.2 / 2296.3 / 2439.8**
  ops/sec; **median 2160.2 ops/sec**.
- Rust fsync-always native server: **5326.0 / 5148.6 / 4944.8 / 5042.7 /
  3260.4** ops/sec; **median 5042.7 ops/sec**.
- Atto trial 3 p50/p95/p99: **404 / 595 / 2091 microseconds**.
- Atto trial 5 p50/p95/p99: **377 / 455 / 1625 microseconds**.
- Median throughput ratio **0.4284**; **not comparable durability**, and Atto
  performance was still increasing over the trials.

The measured Atto process was a real executed actor guest, not a Java/Rust
KV substitute, although the TCP transport and IPC layer are hosted in JDK21.
A direct compiled Atto native networking/storage performance comparison has
not been executed.


## Hardened checked benchmark and stable-keyspace trial reset — 2026-11-11

Source test PR: https://github.com/ores-truffle-oreslang/atto-redis-clone.atto/pull/10
Exact Rust benchmark blob: `c1cde39dc6518e0a9f5def9acc48ffe9bc61ea52`

Seven new binary/frame/oracle/median tests and three additional trial-reset/quota tests
bring the checked Rust workload client from 4 to **14 passing unit tests**.
Even-count medians now use the mean of the two middle ordered values, and reject
nonfinite and nonpositive throughput samples.

The initial expanded 10-trial workload (https://github.com/voxletra/voxletra-e2e/actions/runs/43093181241)
**FAILED** in Atto trial 6 after five successful Atto measured trials. Unlike
the short baseline, this workload created a new trial key namespace on an
unchanged persistent Atto Map without clearing old keys. The runtime's
`OresKeyedMap.MAX_ENTRIES` is 4,096, and the unbounded retained
test namespaces can exceed that limit. The runner's truncated actor stderr
did not preserve the original exception's root message, so classify that
failure's cause as *suspected quota accumulation*, not confirmed.

The benchmark was corrected to issue **correctness-checked DEL commands
for every remaining test key after stopping each trial's timer**.
Every missing/inaccurate/malformed cleanup acknowledgement now fails the run,
rather than being ignored. Both endpoints receive identical cleanup requests.
A regression test explicitly demonstrates that retaining ten independent
trial namespaces without resetting them would exceed 4,096 keys.

**Successful exact-source private CI:** https://github.com/voxletra/voxletra-e2e/actions/runs/43094013417
Carrier SHA `e20e5de051b45d9ee42e6c7e8de440cead9a5f1d`,
job `130778080697`. Exact compiler+guest and Rust benchmark
Git blobs verified; no cross-org token checkout.

- Actual Atto JUnit **4/4** passed, zero skipped/failed; actual actor TCP RESP2
  exchange passed with binary key/value and absent versus empty semantics.
- Java RESP admission bounds **17 passed**; process protocol **29 passed**.
- Rust checked benchmark **14/14 passed**, no failures.
- **Ten trials per endpoint** of 1,000 warm-up + 2,000 measured + **758
  verified post-trial key deletions** each, for 37,580 correctness-checked
  operations per server (20,000 measured).
- Atto actor + JDK21 bridge trial rates: 1802.5, 2441.8, 3084.9, 4000.2,
  4282.9, 4504.9, 4515.0, 4612.0, 4564.2, 4583.7 ops/sec.
  **Median 4393.9 ops/sec**, trial 10 p50/p95/p99 210/244/269 microseconds.
- Rust native fsync-always trial rates: 5250.3, 5352.4, 7278.9, 7209.3,
  6906.1, 6902.4, 7128.3, 7014.0, 7157.9, 7378.1 ops/sec.
  **Median 7071.1 ops/sec**.
- Diagnostic median ratio **0.6214 (62.14%)**.
  This is NOT an Atto-vs-Rust native parity ratio: the Atto server lacks WAL
  durability and uses JDK-hosted TCP+hex IPC while Rust fsyncs every write.
  The test only verifies the specific interpreter/actor/bridge configuration.

The current Atto process performs blocking stdio line reads and its JDK transport
uses blocking sockets; this does **not** satisfy the eventual
Atto-native nonblocking I/O / NIO design requirement. Native Atto async TCP,
durable WAL/SST, sealed actor state and full cross-backend proof remain blockers.
