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
explicitly documented architecture: NIO2's bounded incremental native codec
produces hex strings, while the Atto actor continues to own the sole Map and
processes each command in **one typed actor mail**. There is no Java-side
KV substitute or blocking I/O on this server route. That is a **host-framed
transport optimization**, not an Atto-written RESP2 parser or zero-copy
Bytes design. Treat its binary/stream conformance and performance as pending
until an exact-blob guest execution gate passes.

Current restrictions: one client, ephemeral data, no actor-state sealing proof,
no WAL/AOF/fsync durability, no Native Image AOT, no LLVM/.NET/Wasm/Forge parity.
Benchmark clients may use blocking socket calls; production Atto and NIO2
runtime code may not.
