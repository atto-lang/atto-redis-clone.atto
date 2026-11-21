# Atto Redis async-only I/O contract (stage 1)

## Required production rule

**Only asynchronous I/O may occur on an Atto execution path.** Atto actors
must suspend with `await`, never park scheduler, mailman or CONTROL carriers.
The Java backend must use actual NIO2 completion-driven TCP; wrapping
blocking `Socket`, `InputStream`, `BufferedReader`, `Future.get` or
`Selector.select` inside a host thread is not enough for async-only
certification.

Required callable shape (Atto naming is snake_case):

```atto
pub async routine echo_once(): void {
    val listener = await network.listen_async("127.0.0.1", 16397);
    val conn = await listener.accept_async();
    val bytes = await conn.read_bytes_async(4096);
    await conn.write_bytes_async(bytes);
    await conn.close_async();
    await listener.close_async();
}
```

The compiler implementation is in a separate stacked draft against Java
PR #2883. The Java backend's v1 listener is loopback-only, has one
outstanding accept/read/write per direction, enforces read/write bounds
of 1 MiB, and operates through JDK `AsynchronousServerSocketChannel`,
`AsynchronousSocketChannel` and callback-settled Atto futures.

## Evidence boundaries

- `bridge/AttoRedisAsyncNioServer.java` is an actual asynchronous NIO2
  TCP RESP2 **transport**. Its `AsyncActorBackend` contract accepts a
  `CompletionStage<byte[]>`, never an immediate synchronous KV reply.
- `bridge/AttoRedisAsyncNioServerTest.java` drives an asynchronous mock
  backend, binary/pipelined SET/GET/DEL, fragmented headers, slow-client
  isolation, request limits, backend failure and remote-bind rejection.
  This validates only the NIO2 transport, **not actual Atto execution**.
- `guest/redis_actor_stdio.atto` and `bridge/AttoRedisRespBridge.java`
  are **legacy**. The guest uses synchronous stdin reads, and its bridge
  uses blocking Java sockets and blocking child-process pipes. Do not
  deploy or relabel these as async-only.
- A compiler-backed Atto `listen_async/accept_async/read_bytes_async/
  write_bytes_async` guest must execute before declaring async network
  primitives operational, and a full Atto actor Redis storage guest must
  use the new path before declaring Redis async-only.

## Hardening and performance gates

1. No blocking `java.net.Socket`, `ServerSocket`, `InputStream`,
   `OutputStream` or `Future.get/join` in native asynchronous transport.
   These remain permitted in test *clients*, not the Atto runtime path.
2. Enforce coarse **NETWORK** capability plus scoped NET permission at
   listen admission; loopback only until authenticating remote clients.
3. Every accept/read/write has a single outstanding owner, bounded buffer
   size, deadline, and cancellation/retirement semantics. Closing a
   resource must prevent late completion from resurrecting it.
4. Prove ordered SET/GET/DEL actor handling, absent-versus-empty and binary
   values, pipelining, fragmentation, 32-client bounds and overload
   behavior using the **real Atto compiler and actor**, not a Java HashMap mock.
5. The 1 MiB `List<int>` initial byte API incurs boxing overhead;
   production performance requires a bounded borrowed/owned ByteBuffer
   or Bytes value with audited ownership, Sendable boundaries, and
   zero-copy where safe.
6. WAL writes, fsync, manifest publication and recovery must use asynchronous
   file operations or off-reactor completion queues; no actor state loan
   may survive an I/O await. A synchronous `FileChannel.force` on an
   actor carrier is prohibited. Persistent acknowledgments must still
   preserve the actual durability barrier.
7. Benchmark Atto NIO server and Rust under equivalent durability,
   pipelining, connection counts, value sizes, shard counts and warmup;
   record true Truffle optimizing-runtime status rather than inferring
   it from the JDK version.

## Performance facts before native async Redis

- Correctness-checked Atto actor behind legacy JDK21 IPC bridge:
  median 2,160.2 ops/s (five 1,000-op trials).
- Same legacy actor/bridge on JDK25 hosted runner: median 2,496.3 ops/s;
  that JDK25 run still reported **interpreter fallback**.
- Neither run exercises this new NIO transport or Atto-native async Redis.
  The legacy actor is ephemeral and Rust uses WAL fsync=always; no fair
  native Atto-versus-Rust language throughput ratio is established.

No source org GitHub Actions minutes or unsafe public mirrors are required:
use private approved funded-carrier branches with exact source blob SHAs
and nonzero executed JUnit receipts.


## Executed all-NIO Atto Redis gate (2026-11-11)

The legacy JDK TCP + blocking child-stdio bridge remains only for compatibility
testing. It is **not** the native async server and cannot be relabelled
async-only. New native Atto guests must use the NIO2 capability runtime:
`network.listen_async`, `listener.accept_async`, socket
`read_bytes_async` / `write_bytes_async` (or bounded NIO2
`read_resp2_hex_async` / `write_resp2_hex_async`), and `close_async`.
No synchronous socket, stdin pipes, or blocking `Future.get` on actor carriers.

- Compiler-backed [exact-source run #43108139883](https://github.com/voxletra/voxletra-e2e/actions/runs/43108139883): **12/12 real Atto compiler/Truffle NIO guest JUnit tests pass**; no failures, errors, or skips. Real actor binary RESP2, oversized/malformed rejection, and hash-collision binary-key identity were exercised.
- Native Atto actor guest source blob: `e64e6241403762b6afb460b34b9c2a602ccd1c2e`.
- [Same-runner run #43108684509](https://github.com/voxletra/voxletra-e2e/actions/runs/43108684509): **3 trials × 600 measured TCP requests** each; 50% SET / 40% GET / 10% DEL, binary keys and 64-byte values, 64-key fixed space, 200 warmups/trial, oracle-checked replies, per-trial cleanup (outside timer). Original 1KiB-read NIO Atto median **270.3 ops/s**; fingerprint+4KiB-read NIO Atto **302.6 ops/s**, up **11.97%** in this short diagnostic. Same host official Redis 7.0.15 with AOF/RDB disabled measured **51,218.8 ops/s**. They are different server architectures; this is not native optimized Truffle/AOT parity.
- The stronger result is a bottleneck finding: the guest's per-byte RESP2 state machine and boxed `List<int>` are expensive under JDK21's reported interpreter fallback. Tuning the Map alone will not close the gap.

The optional `guest/redis_actor_nio_fast.atto` takes a different,
**executed and verified** architecture: NIO2's bounded incremental frame codec
produces hex strings, while the Atto actor owns the sole `Map<string,string>`
and processes each command in one typed incoming actor message. There is no
Java-side KV substitute, child stdio, or blocking socket on this server path.
A successful GET replies with a status and a second payload mail.

### Real framed-NIO result (same runner)

[Exact-source private CI #43110995803](https://github.com/voxletra/voxletra-e2e/actions/runs/43110995803)
completed **SUCCESS**. The 12/12 real compiler/Truffle guest tests,
2/2 additional throughput/binary-pipeline JUnit tests, and standalone
async NIO host transport contract tests passed; no failed or skipped JUnit
tests. Fast guest blob: `fb5f2480846417aac88513aaa166b1eeb3a47538`;
original guest blob `6af96f78497268bac71e52b55dfa085ac6cd35ab`;
fingerprint guest blob `e64e6241403762b6afb460b34b9c2a602ccd1c2e`.

Ten commands per block (50% SET / 40% GET / 10% DEL), 64-byte binary
values, 64 bounded binary keys, one sequential TCP connection, 200 warm-up
+ 600 timed operations per trial, three trials/endpoint, with every reply
checked against a local oracle and all remaining keys correctly deleted
outside timing. Official Redis 7.0.15 and Atto were both **ephemeral**.

| Endpoint | Median ops/sec, 3 trials |
| --- | ---: |
| Atto guest RESP2 parser, old 1 KiB receive | 279.6 |
| Atto guest RESP2 parser, fingerprint + 4 KiB receive | 324.6 |
| **Atto-owned KV behind bounded NIO2 framed codec** | **4,893.2** |
| Official Redis 7.0.15, AOF/RDB disabled | 56,077.2 |

Framed-NIO measured **15.08×** the fingerprint/parser configuration
on this CI runner; Redis still measured **~11.46×** the framed Atto
throughput. In the framed actor's third trial, p50/p95/p99 were
171/234/281 microseconds. It is a host-framed transport optimization,
**not** an Atto-written RESP2 parser, zero-copy Bytes proof, or a
native AOT language-speed ranking. The result is one shared-runner
diagnostic sequence with short trials and interpreter fallback; it needs
repeat independent and alternating-order benchmarks.


Current restrictions: one client, ephemeral data, no actor-state sealing proof,
no WAL/AOF/fsync durability, no Native Image AOT, no LLVM/.NET/Wasm/Forge parity.
Benchmark clients may use blocking socket calls; production Atto and NIO2
runtime code may not.


## Single actor-output RESP2 fast path (executed 2026-11-11)

The optional `guest/redis_actor_nio_single_reply.atto` keeps exactly the
same **Atto actor-owned** map, command input semantics, and NIO2 listener as
`redis_actor_nio_fast.atto`. On successful GET it reduces two actor-output
messages (status followed by the hex-encoded payload) to one
`VALUE:<lowercase hex>` envelope. New typed
`socket.write_resp2_reply_async(string)` validates the bounded envelope in
the Java NIO2 codec and writes the resulting RESP2 bytes using
`AsynchronousSocketChannel` completion callbacks. It is **not** a blocking
socket or Java-owned KV shortcut.

[Exact-source funded carrier JDK 21 CI #43120760090](https://github.com/voxletra/voxletra-e2e/actions/runs/43120760090)
completed successfully: **14/14 compiler/codec JUnit tests + 3/3 real-Atto
guest/perf tests**, no skips, with binary/pipelined reply and malformed/
oversized reply-envelope checks. 3 trial medians with 200 warm-up and 600 timed
requests per trial, 64 binary keys, 64-byte values, 50/40/10 SET/GET/DEL:

| Endpoint | Median ops/s |
| --- | ---: |
| Previous framed NIO2 / two actor outputs for GET | 2,843.5 |
| **NIO2 / single actor output for GET** | **3,441.1** |
| Official Redis 7.0.15, RDB/AOF disabled | 19,634.1 |

The same-runner **+21.0%** is a short diagnostic result. **It is not stable
across hosted runners**: [Oracle GraalVM 25 #43121431195](https://github.com/voxletra/voxletra-e2e/actions/runs/43121431195)
measured previous NIO2 **2,794.2** versus single-output **2,866.3**
ops/s (**+2.6%**) on a separate runner. Both have correct RESP replies.
That GraalVM run **still used the fallback interpreter**: installed GraalVM
compiler `25.0.4` is incompatible with Polyglot `25.3.4.1`.

[Separate version-alignment CI #43121854063](https://github.com/voxletra/voxletra-e2e/actions/runs/43121854063)
tested `-Dgraalvm.version=25.0.4` with the same immutable source tree. It
**failed at compilation**, as that older Polyglot API lacks
`Context.Builder.spawnIsolate(boolean)` required by
`IsolatePolicy.java`. Do **not** remove the isolation gate or disable
version checking to obtain misleading throughput numbers.

**Next gates:** obtain an actually compatible optimizing GraalVM/Polyglot
combination while preserving `spawnIsolate`; run longer alternating-order,
multi-runner warm steady-state A/B benchmarks; develop an actor-owned binary
`Bytes` type to eliminate hex allocations; then implement multi-client
async-NIO backpressure and durable asynchronous WAL. Keep these changes
DRAFT until the larger runtime and security contracts are proven.


## Exact-source end-to-end owned-Bytes Atto Redis (2026-11-11)

The optional `guest/redis_actor_nio_owned_bytes.atto` runs a **real compiled
Atto actor** with the runtime-owned `Map<Bytes,Bytes>`. It preserves byte-exact
NUL/invalid UTF-8/CRLF/empty-key-value semantics, single actor response mail,
and callback-only `AsynchronousSocketChannel` through the NIO2
`read_resp2_binary_async` and `write_resp2_binary_async` API. Unlike the
older framed guest, it does **not** use hex-encoded key/value strings.

The accepted binary parser copies accepted key/value bytes once from the
reused receive buffer into immutable `OresBytes` instances, retaining bytewise
equality/hashing, key-byte quotas, actor-mail accounting, and physically
detached copies across private actor boundaries. This is allocation-reduced,
**not** zero-copy. Framing, NIO and RESP encoding remain the Java backend's
responsibility; KV state is owned solely by the Atto actor. The actor declares
`StreamOperator<Tuple[int,Bytes,Bytes],Tuple[string,Bytes]>`; the compiler
derives output types exclusively from that contract, and the independent
ownership pass verifies the binary Map domain. No unsafe cast or skipped
borrow/ownership check was needed.

[Exact-source funded carrier SUCCESS run #43127930267](https://github.com/voxletra/voxletra-e2e/actions/runs/43127930267):
verified SHA-256 pinned GraalVM CE25.3.4.1, optimizing runtime capability,
zero fallback warnings, 15/15 JUnit across binary codec, Map domains and real
Atto NIO/Redis tests; zero skipped/errors/failures. All binary-pipelined
responses passed the oracle. The host-only mock client/test sources use
blocking Java sockets, but neither the guest nor the production NIO2 runtime
does.

Same-runner median of 3 trials × 600 timed sequential ops, 200 warm-up, one
client, 64 byte-key slots, 64-byte values, SET 50% / GET 40% / DEL 10%; Redis
7.0.15 and Atto both ephemeral (Redis AOF/RDB off):

| Endpoint | Median ops/s |
|---|---:|
| Original fully Atto bytewise parser | 220.5 |
| Fingerprint Atto bytewise parser | 250.4 |
| Java-framed NIO2 + Atto Map<string,string> actor | 2686.1 |
| NIO2 + Atto single-output hex actor | 2946.9 |
| **NIO2 + Atto actor Map<Bytes,Bytes> without hex** | **3052.6** |
| Official Redis | 21199.6 |

The owned-Bytes path was 1.036× the single-output hex path and 1.136× the
original NIO2 framed path on this runner. **This is diagnostic, not stable
proof of a broad production speedup**: hosted CPU variance and short
measurement windows are substantial, and the architecture differs from
Redis. The binary path is still ~6.9× slower than Redis here.

**Explicit limitations:** one client, sequential request processing,
in-memory only, no WAL/replication/TTL, no multi-backend or Native Image
conformance, and no stress-tested cancellation/backpressure or hard real-time
latency evidence. Older blocking stdio compatibility bridge files still
exist in the repository and were not executed by this guest. Continue with
longer randomized A/B runs and actor-scheduler/mailbox profiling before
merging the optional fast path.
