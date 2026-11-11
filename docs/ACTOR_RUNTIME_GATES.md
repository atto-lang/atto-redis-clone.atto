# Native AttoLang Redis correctness audit

**Rust-only executable reference under rust/**. The .atto source files are proposed compiler/runtime interfaces and remain **NOT_COMPILED**; none of Java/GraalVM, LLVM, Wasm, Forge or .NET have executed native Atto Redis tests.

- Write linearization: per-shard **whole-command FIFO** through a suspended WAL append. Mere serial actor marking does not prove non-reentrant execution. A bounded SerialCommandLane is REQUIRED before this source becomes executable.
- Durability acknowledgments: always/fsync acknowledges durable state; everysec accepts writes but may lose recently acknowledged operations on power failure. Distinguish WalAck accepted/durable.
- State confinement: read/write only through handle_state critical callbacks; do not await disk I/O with a state borrow. Freeze/copy immutable handles under lock then read outside.
- Move safety: rt take transfers ownership, rt borrow is synchronous only; reserve memory before irreversible WAL append or fail closed and restart/replay after application failure.
- Tombstones: represent deletion as KvEntry(lsn,None), not map.remove; compaction can discard tombstones only after including all older visible segments.
- Durability cut: pause and drain ALL in-flight shard commands through apply; sync WAL barrier, freeze generations, fsync SST, atomically publish/fsync global manifest, then prune WAL <= globally durable cut. Never prune using max(shard watermark).
- Backpressure: bound admission, key/value sizes, pending immutable generations, resident SST indexes, disk occupancy and network clients.

Native acceptance: real compiled .atto SET/GET/DEL, binary empty/tombstone correctness, concurrent same-key WAL order across await, actor loan escapes rejected by compiler, forced-crash replay and torn WAL CRC, sparse-shard checkpoint, failure injection after each fsync/rename boundary, exact compiler and source SHAs. No comparative Atto throughput result may be claimed without executed guest receipts.

The Rust reference benchmark can measure a workload, but it does not measure AttoLang.
