# Native AttoLang Redis correctness audit

**Rust reference under rust/** is the only executed durable RESP2 TCP server. The actual Java/GraalVM compiler and Truffle runtime DID execute `src/redis_kv_smoke.atto` (typed in-memory SET/GET/DEL) and `src/redis_actor_guest.atto` (real actor message handling). Latest hardened Java guest + compiler regression gate: https://github.com/voxletra/voxletra-e2e/actions/runs/43041735134 — 25/25 tests passed. **Only the full `src/kv_database.atto` and `src/checkpoint.atto` storage contracts remain NOT_COMPILED**; they still fail current parser admission and require missing storage, sealed-state and TCP APIs. LLVM, Wasm, Forge and .NET have not executed the Redis Atto server.

- Write linearization: per-shard **whole-command FIFO** through a suspended WAL append. Mere serial actor marking does not prove non-reentrant execution. A bounded SerialCommandLane is REQUIRED before this source becomes executable.
- Durability acknowledgments: always/fsync acknowledges durable state; everysec accepts writes but may lose recently acknowledged operations on power failure. Distinguish WalAck accepted/durable.
- State confinement: read/write only through handle_state critical callbacks; do not await disk I/O with a state borrow. Freeze/copy immutable handles under lock then read outside.
- Move safety: rt take transfers ownership, rt borrow is synchronous only; reserve memory before irreversible WAL append or fail closed and restart/replay after application failure.
- Tombstones: represent deletion as KvEntry(lsn,None), not map.remove; compaction can discard tombstones only after including all older visible segments.
- Durability cut: pause and drain ALL in-flight shard commands through apply; sync WAL barrier, freeze generations, fsync SST, atomically publish/fsync global manifest, then prune WAL <= globally durable cut. Never prune using max(shard watermark).
- Backpressure: bound admission, key/value sizes, pending immutable generations, resident SST indexes, disk occupancy and network clients.

Native acceptance: real compiled .atto SET/GET/DEL, binary empty/tombstone correctness, concurrent same-key WAL order across await, actor loan escapes rejected by compiler, forced-crash replay and torn WAL CRC, sparse-shard checkpoint, failure injection after each fsync/rename boundary, exact compiler and source SHAs. No comparative Atto throughput result may be claimed without executed guest receipts.

The Rust reference benchmark can measure a workload, but it does not measure AttoLang.


## Confirmed Java/GraalVM language safety fixes (2026-11-11)

Actual compiler probes first showed mutable Map callables could escape as values, an unsupported Map<int,int> parameter could call set, and a const Map variable could mutate. Source Java PR #2881 now rejects:
- captured first-class Map.set/Map.delete member values (direct-call-only);
- unproven scalar key/value domains or nominal Map shadows;
- const, val, borrowed and temporary mutation receivers.

It retains direct calls on writable let locals and writable actor-owned fields, and use-after-rt-take rejection. The tests are run by the actual Atto Java compiler/Truffle guest, NOT Rust or host-only models. A writable let is not yet a complete global exclusive-alias proof. No sealed actor-state loan parity is claimed.

## Rust RESP2 reference security findings (2026-11-11)

- The former parser limited only each bulk using the maximum configured storage value size, allowing many bulk arguments to cumulatively allocate far beyond a modest request budget. It now caps each bulk to 4 MiB and the entire RESP array frame to 8 MiB, including line/trailer bytes, rejecting before allocation.
- Command names are capped at 32 bytes; argument count remains at most 16; client slots remain 64 and are released using a panic-safe guard.
- An unauthenticated server cannot bind a non-loopback IP through CLI **or** through the public serve() library API. No remote-listening override exists without future authentication/TLS.
- Rust security tests cover too-large bulk and command header early rejection, cumulative length charging, CLI and library remote binding rejection, alongside WAL/SST/restart and RESP tests.

Rust reference CI is separate from compiled Atto guest evidence; neither test stream establishes the complete Atto Redis native server.
