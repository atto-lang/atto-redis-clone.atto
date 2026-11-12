# Official Redis 7.0.15 vs AttoLang actor: executed performance baseline (2026-11-11)

## Verdict and evidence

**Executed, PASS:** [private exact-source funded carrier run #43106350960](https://github.com/voxletra/voxletra-e2e/actions/runs/43106350960), job ID `130810582327`, workflow commit `ee77233e2f6f0dd18629d1c7c2a57ed1b2a28412`, branch `validation/atto-vs-official-redis-20261111`.

This is a head-to-head *RESP2 end-to-end* measurement of official Redis versus an **actual AttoLang actor guest** reached through a Java 21 TCP/stdio/hex IPC bridge. The guest owns the KV Map; Java and Rust do not substitute a KV store on the Atto path.

**It is not an Atto-native TCP implementation, async-only Atto transport benchmark, native-image performance measurement, zero-GC proof, or durable Redis-equivalence test.** The comparison is diagnostic; the servers have different networking/IPC and scheduler architectures.

## Exactly what executed

- Official Redis `redis-server v=7.0.15`, Ubuntu package `5:7.0.15-1ubuntu0.24.04.5`, bound to `127.0.0.1:26379`, `--save "" --appendonly no` (ephemeral; neither RDB nor AOF).
- Atto guest `redis_actor_stdio.atto` via the actual Java/Truffle compiler guest and `AttoRedisRespBridge`, bound to `127.0.0.1:26381`; ephemeral Map with 4,096-entry quota.
- Same GitHub-hosted Ubuntu 24.04 runner, reported Linux kernel `7.0.0-1017-azure`, two allocated vCPUs, AMD EPYC 7763 CPU model; Temurin OpenJDK 21.0.13.
- A single sequential RESP2 client connection per endpoint, no pipelining; 50% SET, 40% GET, 10% DEL, 64-byte binary values, 1,024-key potential working set.
- Ten trials/endpoint; 1,000 warm-up requests + 2,000 timed requests per trial; 758 correctness-checked post-trial DEL commands per endpoint per trial, outside timing. Therefore 37,580 checked requests per endpoint and 20,000 timed requests per endpoint.
- Every SET/GET/DEL reply validated against the client oracle; all ten trials and all trial cleanups passed. No reported false-success errors.
- Existing compiler-backed Atto guest suite 8/8, RESP bridge bounds 17 checks, bridge contract 29 checks; checked Rust benchmark client tests 14/14 passed in the same run.
- Source compiler baseline and mirrored exact source file blob hashes verified *before* running tests. Atto guest blob `c777f0ca9de51febacbf91d350e329c53614b059`; benchmark client blob `c1cde39dc6518e0a9f5def9acc48ffe9bc61ea52`. The benchmark workflow was the only difference from the earlier, successful compiler/guest carrier baseline.

## Results

| Measure | Official Redis (memory-only) | Atto actor (JDK21 TCP + stdio bridge) |
| --- | ---: | ---: |
| Median throughput, ten trials | **20,957.0 ops/s** | **3,163.3 ops/s** |
| Trial 10 p50 | 44 µs | 293 µs |
| Trial 10 p95 | 61 µs | 339 µs |
| Trial 10 p99 | 73 µs | 388 µs |
| Correctness-checked trials | 10/10 | 10/10 |

Atto/Redis median throughput ratio: **0.1509** (15.09%); Redis/Atto: **~6.63x**.

### Per-trial rates (ops/s)

| Trial | Redis | Atto |
| ---: | ---: | ---: |
| 1 | 20,918.7 | 1,464.5 |
| 2 | 21,151.6 | 2,043.2 |
| 3 | 20,552.6 | 2,476.3 |
| 4 | 21,399.3 | 2,962.9 |
| 5 | 21,174.5 | 3,214.5 |
| 6 | 20,763.7 | 3,156.2 |
| 7 | 20,847.5 | 3,170.5 |
| 8 | 20,709.7 | 3,263.1 |
| 9 | 21,247.2 | 3,260.0 |
| 10 | 20,995.3 | 3,276.6 |

**Label caveat:** The existing checked-client CLI uses `--rust` as the first endpoint argument, and prints `rust_fsync_always` as a legacy *label* even when the endpoint is official Redis; the CI step explicitly points that parameter at Redis `127.0.0.1:26379` and logs the mapping. The **results above are from official Redis, not the Rust clone**. The subsequent `atto_actor_bridge_ephemeral` endpoint points at Atto on `127.0.0.1:26381`.

The same carrier additionally benchmarked the durable Rust reference (`fsync=always`): median 5,106.0 ops/s versus Atto median 3,261.4 in that separate sequence. It is not durability-comparable and is **not** the Redis comparison.

## Reproduce the exact tested setup

The CI workflow at carrier commit `ee77233e` verifies pinned source Git blobs, compiles/tests the real Atto compiler guest and TCP bridge, installs official Redis, starts both memory-only loopback servers, and runs the checked client:

```sh
# In the verified carrier after the build and both servers have started:
./rust/target/release/bench_resp_checked \
  --rust 127.0.0.1:26379 \
  --atto 127.0.0.1:26381 \
  --requests 2000 --warmup 1000 --trials 10
```

Use the run URL above to inspect the actual runner logs rather than inferring numbers from this document.

## Limits and follow-up gates

1. **Benchmark uncertainty:** This is one GitHub-hosted run, with ten short sequential trial samples. Actor warm-up is significant; measurement order favors Redis first and then an Atto interpreter still warming. Repeat on pinned dedicated hardware with alternating/interleaved trial order and longer steady-state windows. Do not claim production or language-intrinsic performance from this ratio.
2. **No asynchronous I/O claim:** This executed bridge uses blocking child stdio/hex encoding and a Java TCP listener. The newer NIO2 async transport *contract* is not wired to the executing Atto guest in this benchmark. Wire fully async I/O and remove avoidable process-IPC copies before attributing bottlenecks to Atto.
3. **No durability:** Redis RDB/AOF is disabled to match Atto's ephemeral Map. For an equivalent durable comparison, implement Atto WAL/SST/recovery and match Redis AOF fsync settings; separately run fault-injection and crash/restart tests.
4. **No multiclient scalability:** This measures one sequential connection. Add throughput and p50/p95/p99 for concurrent 1/8/32 clients, pipelining on/off, GET-only, SET-only and 50/50, with identical key sizes and allocation policies.
5. **No native compilation parity:** JDK21 interpreter only. Re-run when GraalVM Native Image or LLVM compiled Atto server is executable, and preserve exact compiler/guest SHAs, host CPU/OS, runtime mode, RSS/CPU usage and response checks.

See [Atto actor performance history](ATTO_ACTOR_PERFORMANCE.md) for separate hosted-runner Atto versus **Rust reference** benchmarks.
