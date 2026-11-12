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


## JVM-owned Bytes and GraalVM 25 verification — 2026-11-11

The compiler runtime draft [Java PR #2894](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2894)
now includes \`OresBytes\`: bounded (1 MiB), immutable, single-owner
transfer by \`take()\`, use-after-take prevention, read leases and
deferred backing-store zeroization while trusted NIO2 reads are active.
The RESP2 codec has a new \`parseBinary\` / \`encodeBinary\` path, which
validates the same strict byte framing as the existing string path but
does not create hex strings or \`List<int>\` boxes for payloads.

**Evidence:** [exact-source CI #43124972780](https://github.com/voxletra/voxletra-e2e/actions/runs/43124972780):
21/21 executed JUnit cases, zero errors/failures/skips, including
7 newly added \`OresBytesBinaryCodecTest\` cases (ownership transfer,
racing takes, use-after-take, lease retention during retirement,
all-octet binary identity, fragmented/malformed RESP2 and nil/empty).
Both real Atto Redis actor variants remain green.

**Integration status:** this is a *JVM runtime foundation*. The currently
executed guest still uses hex-string actor messages, so the new binary
path has NOT yet produced an Atto Redis throughput improvement, has
NOT yet been exposed as a validated first-class Atto \`Bytes\` type,
and does NOT yet prove cross-backend or actor transport parity.

The exact matching [GraalVM Community 25.3.4.1 release](https://github.com/graalvm/graalvm-ce-builds/releases/tag/graal-25.3.4.1)
was installed from its official tarball and verified against its
published SHA-256, rather than taking a potentially incompatible
generic JDK25 GraalVM build.
[Executed run #43124110196](https://github.com/voxletra/voxletra-e2e/actions/runs/43124110196)
passed 10,000 checked requests after 5,000 warmups at
3,540.5 ops/sec; [repeat #43124972788](https://github.com/voxletra/voxletra-e2e/actions/runs/43124972788)
reported 5,540.7 ops/sec on a later hosted runner.
Neither run emitted a fallback-runtime warning. **No Truffle \`opt done\`
compilation event was observed**; compatible GraalVM JDK execution is
not proof that the existing \`OresEvalRootNode\` boundary-heavy guest
evaluator was optimized by Truffle.

Do not directly divide the GraalVM 10k/5k-warmup results by the
Temurin JDK21 1k/200-warmup results and label it a JIT speedup;
the workload duration, warmup and runner vary. A paired,
identical-workload same-machine comparison and actual hot-call-target
compilation receipt remain required. Full zero-copy actor transport
needs typed guest \`Bytes\`, linear ownership across mailboxes,
bounded per-connection socket writes and explicit cancellation leases.


## First-class Atto Bytes + safe NIO2 handoff checkpoint (2026-11-11)

**Executed [CI #43143141878](https://github.com/voxletra/voxletra-e2e/actions/runs/43143141878): 27/27 compiler-backed JUnit tests PASS**, zero errors/failures/skips, on the exact Java compiler and runtime blob SHAs. This includes the actual compiled Atto \`Bytes\` guest plus the original and one-message Redis actor guests.

The Java source draft now has:

- Compiler-resolved \`Bytes\` (non-generic) and synchronous \`size()\`, \`byte_at(int)\`, \`take()\` and \`close()\` members. In particular, \`Bytes.size()\` does **not** return \`Future<int>\`—a real compiler test caught and corrected the initial type mismatch.
- \`NioConnection.read_owned_bytes_async(int)\`: NIO2 callback read into bounded immutable owned bytes (no \`List<int>\` boxing); clean EOF yields empty bytes; zero-byte completions rearm rather than faking EOF.
- \`NioConnection.write_owned_bytes_async(Bytes)\`: compiler-checked linear consumption, corresponding runtime O(1) \`take()\`, immutable read lease held until all kernel write completions or cancellation; handle invalidation and final zeroization are exact-once. The guest cannot reuse the transferred value.
- \`OresFuture.Callback.tryResolve(T)\`: atomic resource-publication acknowledgment, so a cancellation racing with a late NIO read completion cannot silently discard an owned buffer without closing it. Two additional tests verify cancellation/acceptance disposition and backing-array erasure.
- Pinned \`maven-resources-plugin:3.3.1\` in the source Maven POM to avoid transient default-resolution behavior on funded runners. Exact POM SHA is checked by the private carrier.

The running Redis actor still has the transitional hex-based mailbox encoding. The new linear \`Bytes\` protocol has been proven over *one real Atto NIO socket*, **not** through shared/private actor mailbox admission. Actor transfers need exact-once ownership commit, memory/queue quota accounting, cancellation rollback, and death/stop reclamation before they can be called zero-copy.

**Executed [GraalVM25 tracing run #43143141904](https://github.com/voxletra/voxletra-e2e/actions/runs/43143141904):** 3,594.2 checked ops/sec with 10k measured requests, 5k warmups, release SHA256 verified, fallback warning absent. The real launcher now explicitly accepts \`--trace-truffle\` and enables \`engine.TraceCompilation\` with experimental engine options. **No \`opt done\` event was observed**; this is not an optimized guest-code receipt. Root execution currently delegates to \`@TruffleBoundary\` and needs an actual hot-call-target specialization before claiming Graal JIT speedup.

**Executed [same-runner A/B #43143141842](https://github.com/voxletra/voxletra-e2e/actions/runs/43143141842):** original Atto NIO Redis 1,490.1 ops/sec, single-message actor 1,499.2 ops/sec (+0.6% within this run), Rust WAL/fsync-always 5,045.3 ops/sec. The 0.6% difference is negligible compared with runner variation; earlier positive A/B runs should not be represented as a guaranteed speedup. Different durability and warmup configurations prohibit cross-runtime parity conclusions.

**Explicit outstanding gates:** actual observed hot Truffle target compilation and a matched compilation-on/off A/B comparison; actor mail/stream ownership for \`Bytes\`; memory-safe replay/disconnect/cancellation stress; sealed actor state; multi-client NIO backpressure; asynchronous durable WAL/recovery; and exact per-backend execution for LLVM, WASM, .NET and Forge. Keep both development PRs in DRAFT until their own release criteria are met.
