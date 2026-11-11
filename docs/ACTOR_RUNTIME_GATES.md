# Native AttoLang implementation and verification gates

## Semantic design

Each key belongs to one shard. `KvShard` is a `define serial actor`; it owns a map and pending immutable generations. Actor state is only touched by `self.handle_state(critical-read|critical-write ...)`. A WAL actor persists a checksummed record before an acknowledged map change is observable.

**Important:** some AttoLang backends permit actor turns to suspend and overlap. A serial actor's runtime contract must guarantee whole-command order across WAL `await`, or the storage module MUST add an explicit per-shard sequenced FIFO admission queue. Concurrent readers may borrow immutable state, but they cannot observe a committed write out of WAL order.

Memory ownership: `rt take` moves buffers across actor boundaries; `rt borrow` may not survive `await` or escape sealed state. A disk flush receives an immutable/frozen generation by ownership transfer and installs a file descriptor/segment handle only after manifest publication.

Durability order:

1. Append framed WAL record with validated lengths, monotonic LSN and checksum.
2. In always-fsync mode, `fsync` before acknowledging a write.
3. Freeze active map without moving/invalidating clients' outstanding values.
4. Write sorted immutable segment and fsync its file.
5. Atomically replace a versioned manifest, then fsync parent directory.
6. Only after the manifest is durable may WAL records through the checkpoint fence be pruned.
7. During recovery, replay WAL records newer than each shard's durable sequence watermark. Truncate only *incomplete* tail records, fail closed on complete invalid frames.

Compaction must preserve the newest LSN for each key, hold/delete tombstones until all older segments are covered, and avoid deleting files still referenced by a committed manifest.

## Required native modules

- `HashMap<Bytes, Entry>`, `Bytes`, `ImmutableMemtable`, `StableHash.crc32`
- `WalFile.append_and_fsync`, `WalFile.sync_barrier`, `WalFile.prune_through`
- `SegmentStore.write_sorted_segments`, `publish_manifest_atomically`, `fsync_parent_directory`
- `KvDatabase.pause_admission_and_rotate_all` and a bounded command queue
- Async RESP2 TCP reader with strict length and frame limits
- Cross-actor send/receive and ownership-correct file-buffer handoff

These names are **proposed library API contracts**. Merely parsing them does not show they are implemented.

## Conformance requirements

For each of Java/GraalVM, LLVM, Wasm, Forge and .NET: compile real `.atto` fixtures, run them in the guest, verify SET/GET/DEL over sockets or host-admitted channels, forced crash/reopen, checksum errors, WAL truncation, concurrent same-key ordering, ownership negative tests, reactor suspension, release/cancellation, and memory limits. Record immutable source and compiler SHAs, invocation, exit status and observed output. Mark absent guest runs **NOT_EXECUTED**, not PASS.

Python oracle tests provide reference behavior only. LLVM needs ASan/UBSan and deterministic destructor/zero-tracing-GC evidence; Java/.NET need AOT execution and GC profiling. This repo does not currently establish those properties.

## Operational controls

The Python server defaults to checkpointing every 60 seconds and compacting every tenth checkpoint; these run off the asyncio event loop. The WAL uses an OS file lease so multiple processes cannot accidentally share one data directory. Forced-process-exit recovery is tested with an fsynced WAL; OS power-loss durability and native backend execution remain unverified.
