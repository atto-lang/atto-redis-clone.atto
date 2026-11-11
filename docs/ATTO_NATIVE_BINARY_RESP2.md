# Atto-native binary RESP2 Redis milestone (2026-11-11)

## Executed Atto source

`guest/redis_binary_resp2.atto` contains the **application-layer RESP2 parser, command
dispatch, bounded binary storage, and response encoder in AttoLang**. It calls
`network.listen_async`, `accept_async`, `read_bytes_async` and
`write_bytes_async` on the real NIO2 Java/GraalVM compiler candidate.
No Python, Rust or Java Map/RESP fallback is used on this guest path.

The current guest supports one loopback TCP connection per invocation.
It accepts arrays of 1–3 bulk arguments and commands PING, GET, SET, DEL,
QUIT; unknown well-framed commands receive `-ERR unsupported`. Binary keys
and values remain `List<int>`, not strings. It distinguishes NIL GET,
`$0\r\n\r\n` for an empty value, and integer 0/1 DEL results.
A single actor owns a bounded three-list key/value/live store, with 256
entry slots and tombstone reuse. The command is sent once in an
`ActorMail<Tuple[int, List[int], List[int]]>`; replies are typed tuples.
Actor helpers are defined **inside the actor**, respecting the compiler's
same-file ambient dependency prohibition.

The parser checks canonical decimal lengths and CRLF trailers; supports
partial socket reads, pipelining, embedded NUL, non-UTF-8 and CR/LF in
payloads. Caps: three arguments, command 16 bytes, key 4096 bytes,
value 16384 bytes, and aggregate frame 20576 bytes. It rejects malformed
or over-budget frames and closes the sole connection.

## Real compiler and executable evidence

- [Funded, exact Git-blob private run #43104365649](https://github.com/voxletra/voxletra-e2e/actions/runs/43104365649):
  **9/9 actual Java compiler+Truffle JUnit tests passed** (3 async NIO,
  1 spawned actor, 1 fixed-frame Atto Redis, **4 generalized binary RESP2
  Atto actor tests**), zero skipped/errors/failures.
  Exact Atto binary source blob `6af96f78497268bac71e52b55dfa085ac6cd35ab`;
  full runner carrier head `784c4d915cbb2b68eb7370cbed185b6407c37058`.
  Source manifest and changed files checked with Git blob hashes.
  Extra Java host contract 7/7 checks used a mock backend and **must not
  be presented as Atto execution**.
- [Extended exact-head private run #43105033703](https://github.com/voxletra/voxletra-e2e/actions/runs/43105033703),
  private carrier commit `fa79d4611a0df35190f982b53c4031c8d9c7a6fd`:
  **11/11 actual compiler + Truffle JUnit guest tests passed**, including
  **6/6 binary RESP2 actor tests**, zero skipped/errors/failures.
  The extra two executed tests cover a 1024-byte binary bulk, empty/distinct
  binary keys, negative bulk length and bad trailer.
  JUnit test source Git blob `26e584519cdc20c95000ab82feab8abfb68ab374`.
  Real Atto source blob remains `6af96f78497268bac71e52b55dfa085ac6cd35ab`.
  The separate **7/7 standalone Java NIO tests** used a mock actor and
  were explicitly not counted toward real Atto assertions.

## Production and performance exclusions

This is a correctness vertical slice, not a Redis replacement or an honest
Atto-vs-Rust benchmark. No indexed hash Map<Bytes,Bytes> (the actor currently
linearly scans up to 256 entries), native owned Bytes buffers or zero-copy,
multi-client accept loop/connection actors, authentication, TCP backpressure
and lifetime cancellation proof, generic sealed actor state, WAL/fsync and
crash replay, supervisor epochs, LLVM/.NET/WASM/Forge parity or native AOT.
The Java compiler uses an interpreter fallback on this JDK21 carrier.
Do not expose to non-loopback interfaces. Benchmark only after equivalent
durability/transport and pinned optimizing runtime targets.

## Next Atto-language implementation gates

1. Introduce an owned `Bytes` standard core value with bounded nonblocking
   socket byte transfer, audited `Sendable`, borrowing, `rt take`, and
   snapshot semantics. Add typechecking and guest tests.
2. Replace the O(256) parallel array store with actor-owned
   `Map<Bytes,Bytes>` supporting arbitrary binary hashing, bounded capacity,
   duplicate replacement, deletion, and isolation.
3. Split command declarations into an explicitly imported `KvCommand`
   module, preserving actor lexical isolation without privileged same-file
   dependencies. Add negative import/ownership tests.
4. Add one accept loop with bounded independent connection actors, per-socket
   parser budgets, backpressure, cancellation fencing and graceful shutdown.
5. Build a real WAL writer actor with asynchronous append and fsync barrier
   semantics; implement replay and supervisor recovery, fault injection and
   durability-correct acknowledgments.
6. Run fair, reproducible Atto-vs-Rust throughput/latency/CPU/GC measurements,
   equal durability and connection counts, and cross-backend parity gates.

The relevant draft PRs must remain unmerged until their dependencies and
execution receipts are reviewed.
