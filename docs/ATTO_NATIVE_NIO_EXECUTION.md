# Atto Redis real asynchronous NIO2 execution and performance

## What actually executes

The reviewed Atto guest \`guest/redis_actor_nio.atto\` has a real spawned
Atto actor owning a Map<string,string>, with SET/GET/DEL performed exclusively
inside its mailbox handler. The network path uses:

- \`network.listen_async\` and \`listener.accept_async\`
- \`socket.read_resp2_hex_async\` and \`socket.write_resp2_hex_async\`
- \`await\` on every network operation

The host may read the reviewed source file **once at bootstrap**; it does
not read or write Redis requests. The running Atto I/O data path uses Java
\`AsynchronousServerSocketChannel\`, \`AsynchronousSocketChannel\` and
completion callbacks, not blocking sockets or a stdin/process-pipe bridge.

The alternate variant \`guest/redis_actor_nio_single_mail.atto\` uses one
immutable hex-safe \`OP|key|value\` actor mailbox message per command and one
\`VALUE|hex\` reply message per successful GET. Both accept the same RESP2
wire format and maintain binary/missing-versus-empty semantics. This is a
temporary string representation, **not zero-copy**.

## Executed proof

| Receipt | Evidence |
|---|---|
| [#43104065825](https://github.com/voxletra/voxletra-e2e/actions/runs/43104065825) | 13/13 Java/JUnit tests PASS, including actual Atto Redis binary SET/GET/DEL over direct NIO2, parser and cancellation |
| [#43107083760](https://github.com/voxletra/voxletra-e2e/actions/runs/43107083760) | 14/14 tests PASS, including both original and optimized Atto actor guests |
| [#43103454820](https://github.com/benefactor-cc/benefactor-e2e/actions/runs/43103454820) | 49 exact-source RESP2 parser checks PASS in alternate private carrier |

Git objects were mirrored by **exact blob SHA** to private funded Actions
carriers. No public repo copies or cross-org checkout tokens were necessary.
The RESP2 parser originally admitted \`*4\r\n\` despite MAX_ARGS=3 because
signed Java integer division rounded \`(max - digit) / 10\` toward zero.
The actual bug was fixed by checking \`digit > max\` before arithmetic.
Do not weaken the negative fixture to hide the failure.

## Executed performance

Real native async NIO2 Atto actor, JDK21/Truffle **interpreter fallback**,
no WAL versus Rust native WAL with fsync **always**. These are not equal
durability modes. Each trial uses a Rust correctness-checking RESP2 client,
50% SET / 40% GET / 10% DEL, 64-byte values, 1,024 key slots,
200 warmups and 1,000 measured sequential requests. Five trials are
summarized with median throughput and every response verified.

| Run | Original Atto NIO2 | One-message Atto NIO2 | Rust fsync-always |
|---|---:|---:|---:|
| [#43105313706](https://github.com/voxletra/voxletra-e2e/actions/runs/43105313706) | 1,347.7 ops/sec | not tested | 4,021.4 ops/sec |
| [#43107347337](https://github.com/voxletra/voxletra-e2e/actions/runs/43107347337) | 989.5 ops/sec | **1,117.0 ops/sec** | 4,146.5 ops/sec |
| [#43107906718](https://github.com/voxletra/voxletra-e2e/actions/runs/43107906718) | 1,356.0 ops/sec | **1,614.5 ops/sec** | 4,958.1 ops/sec |

Two separate **same-runner A/B runs** showed compact mailbox gains of
12.9% (ratio 1.1289) and 19.1% (ratio 1.1906), respectively. In the latest
run, the checked client also deleted the remaining live keys after each trial;
`checks=1598` counts 200 warmup + 1,000 measured + 398 verified cleanup
requests, and cleanup is excluded from timed throughput. These runs are
observations, not statistical proof of a fixed speedup. The absolute Atto medians differed
across runners/runs: do not claim the language has a fixed speedup or infer
confidence intervals from five trials. A direct Rust-versus-Atto language
performance claim is **not justified** until persistence and concurrency
policies match.

The JDK25 legacy-bridge figure (2,496.3 ops/sec) is **not** an async-NIO
measurement. The host starts fresh single-client Atto instances per trial
because this milestone's guest accepts one connection only.

## Hard release blockers

1. **Single client:** implement a real concurrent NIO2 accept loop, bounded
   per-connection request queues and shared actor dispatch, with ordering
   and overload admission verified under disconnect races.
2. **State safety:** migrate temporary legacy actor fields to sealed actor
   state and authorized critical read/write access. Never hold a critical
   state loan across an I/O await.
3. **Owned byte buffers:** eliminate \`List<int>\` boxing and hex-string
   shuttling, use bounded actor-sendable Bytes and NIO owned/borrowed
   buffers with no UAF/double-free.
4. **Optimizing runtime:** prove actual GraalVM 25 Truffle optimizing
   compilation, not merely Temurin JDK25. Run warmups and record
   interpreter/JIT state per measurement.
5. **Async durability:** WAL append, group commit, nonblocking fsync,
   crash recovery and acknowledgment-after-barrier conformance. Never
   pretend \`FileChannel.force\` on an actor scheduler is async.
6. **Five-backend parity:** compiler+runtime+negative fixtures for Java,
   LLVM, .NET, WASM and Forge. The above receipts prove **Java only**.
7. **Fair benchmarking:** equal durability, key distributions, workloads,
   concurrency, CPU affinity, value sizes and backpressure, plus p50/p95/p99,
   allocation/GC and repeated clean-run receipts.

Compiler feature PR:
[oreslang-source.java #2894](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2894).
The compiled Redis guest is reviewed in
[atto-redis-clone.atto #13](https://github.com/ores-truffle-oreslang/atto-redis-clone.atto/pull/13).

Neither PR should be represented as production-ready until these gates pass.
