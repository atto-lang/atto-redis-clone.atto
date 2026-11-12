# Actual Atto Redis: core/std tracing and RESP/NIO bottleneck analysis (2026-11-11)

**Status:** executed diagnostic measurements on real Atto actor source; not a native AOT,
five-backend, durable Redis, or statistically stable speed-parity claim. This file records
*exact test runs*, important negatives, and the next implementation priorities.

## Instrumentation and ownership

- Atto core `CorePerf` is a bounded, fixed numeric-ID recorder. The disabled hot path
  performs no clock reads, formatting, or allocations. Enabled events are exported as
  JSONL after the JVM exits, not from within actor or socket request handling.
- Real Atto standard library `std/trace` was executed in its own guest tests:
  `Trace.start_ns`, `Trace.duration_ns`, and bounded `Trace.Buffer`. It does not
  synchronously log request payloads or perform network I/O.
- Java/NIO backend probes measure **real** Atto guest's transport work, not a Java KV
  surrogate: RESP scanning, byte-to-hex conversion, reply encoding,
  asynchronous socket write submission and completion. The Atto actor continues
  to own the `Map<string,string>`. The `std/trace` module is tested separately;
  actor-internal queue latency/Map work **has not yet** been timed with it.
- Probe changes live in compiler [PR #2908](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2908),
  stacked on [PR #2904](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2904).
  The original and single-reply actor fixture tests are in compiler
  [PR #2909](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2909).

## Recorded real-source profiling

[Exact private CI #43122033972](https://github.com/voxletra/voxletra-e2e/actions/runs/43122033972)
succeeded. Source compiler and guest Git blob identities were independently checked.
17/17 core/std/Atto guest JUnit tests, the sampled real actor profile, and a
profiler-**OFF** binary-pipeline correctness regression all passed.

| Numeric core phase | Samples | p50 ns | p95 ns | p99 ns |
| --- | ---: | ---: | ---: | ---: |
| `redis.resp.parse` | 2454 | 4008 | 25858 | 50946 |
| `redis.resp.hex` | 1814 | 1534 | 10690 | 26910 |
| `redis.resp.encode` | 1227 | 2485 | 18214 | 36138 |
| `redis.tcp.write.submit` | 1226 | 75992 | 245931 | 770675 |
| `redis.tcp.write` (through completion) | 1226 | 122880 | 295264 | 1128937 |

Submission and completion intervals **overlap** and must not be added together.
The largest observed hot-path intervals are asynchronous write submission/completion,
not byte-by-byte RESP scanning or hex encoding in the framed codec.
This does **not** isolate actor queue wait, VM scheduling, or kernel-only write time.
Profiling-on request rate is not directly comparable with uninstrumented runs.

## Actor mailbox command/reply optimization: correct, no demonstrated speed gain

This PR sends `Tuple[string,string]` (status, hex payload) as a single actor
output for every SET/GET/DEL; GET hits no longer need a second mailbox wait.
The compiler requires `Tuple[string,string]` as a finite local sequence shape;
the old `Tuple<string,string>` and direct indexing into `output.value` were
**rejected by real compiler tests**, and were corrected, not bypassed.

The [real Atto ABBA CI #43122848193](https://github.com/voxletra/voxletra-e2e/actions/runs/43122848193)
passed all real guest and core/std tracing tests. Actual Atto actor throughput
of four consecutive, correctness-checked guest processes:

- Original double-reply: **1,611.6** ops/s (first)
- Modified single-reply: **1,966.5** ops/s (first)
- Modified single-reply: **2,656.9** ops/s (second)
- Original double-reply: **3,062.4** ops/s (last)

Means: original 2,337.0; single-reply 2,311.7 ops/s; ratio **0.9892**.
No demonstrated speed improvement due to warmup/runtime drift. Keep DRAFT.

## Exact matching optimizing GraalVM — not the generic JDK25 selection

Truffle dependency version in `pom.xml` is `25.3.4.1`.
The generic `graalvm-community` + `java-version: 25` action resolved to
GraalVM Community `25.0.2` and emitted interpreter-fallback warning, despite
binary correctness PASS. That run was correctly **failed** rather than claimed
optimized: [#43123354281](https://github.com/voxletra/voxletra-e2e/actions/runs/43123354281).

The source-free private runner was then corrected to download official GraalVM
Community **25.3.4.1** from `graalvm/graalvm-ce-builds` release
`graal-25.3.4.1`, with exact vendor asset SHA-256:
`b2bc38d0c4141426eb44d0eefa3cc172c96faf92727d703b61541699128b6fc7`.
The real Atto binary pipeline test passed with no interpreter fallback warning.
The all-green [#43123826015](https://github.com/voxletra/voxletra-e2e/actions/runs/43123826015)
also executed a separate 3-trial, 600-measured-request-per-trial TCP RESP2 comparison
on the same runner, without persistence:

| Executed server path | Median ops/sec |
| --- | ---: |
| Atto-written RESP2 parser, baseline | 363.6 |
| Atto-written RESP2 parser with fingerprint/batching | 428.6 |
| **Real Atto actor + bounded JDK NIO2 framed codec** | **5,122.8** |
| Official Redis 7.0.15, RDB/AOF disabled | **77,354.1** |

The Atto framed path achieved ~6.62% of Redis throughput in this particular short
sample. **No proof that hot methods actually compiled** (only an optimizing-capable
VM with no fallback warning); do not infer a 25.3-specific speedup by comparing
unpaired JDK21 and GraalVM25 runs on different transient CI hosts.

## Next measured improvements

1. **Instrument request->actor admission->receive->Map operation->reply dequeue**
   separately, using bounded core/std timer events with a correlation ID and no
   untrusted payload. First isolate scheduler queue time vs actor map execution.
2. **Time `OresFuture.fromCallback` and network submission without changing
   cancellation semantics**, then measure allocations/GC, CPU hot methods, and
   context switch rates. Existing NIO2 flow must remain completion-driven;
   never insert blocking I/O or `Future.get`.
3. Run long warm-up and many iterations on the same process with source-pinned
   25.3.4.1 optimizing runtime. Capture actual Truffle compilation events,
   compilation failures/deoptimizations, and p50/p95/p99, not just absence of fallback.
4. Replace transient hex strings with an owned `Bytes` path when the compiler
   enforces transport, lifetime, actor isolation, and binary Map safety. Maintain
   exact RESP bytes, invalid frame rejection, payload limits and backpressure.
5. Verify 1/8/32 clients and pipelined workloads, then parity with equally durable
   asynchronous WAL, fsync acknowledgments, and crash/restart recovery.

**Do not merge** as production, optimized native Atto, or durable Redis parity until
the missing gates execute. Existing source PRs remain draft and stacked.
