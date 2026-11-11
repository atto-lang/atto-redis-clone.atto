# Redis supervisor and crash-recovery contract

Status: **target design / NOT_COMPILED Atto implementation**. `src/redis_supervision.atto`
and the `StorageAdmission` calls in `src/kv_database.atto` are not
verified stdlib/runtime APIs. They do not establish native Atto behavior.
The Rust executable in `rust/` remains a storage/RESP reference, **not**
a supervised actor runtime. The Rust supervision tests model transitions only.

## Ownership and topology

```text
OS/service manager (root actor/process failure => fail-stop and fresh process)
  |
  RedisRootSupervisor : Supervisor [owns exclusive store lease and epoch authority]
  +-- StorageSupervisor : Supervisor [one storage recovery failure domain]
  |     +-- WalWriter (single durable append ordering lane)
  |     +-- KvShard[0..N-1] (one command FIFO lane per shard)
  +-- NetworkSupervisor : Supervisor
  |     +-- TCP listener
  |     +-- client connection actors
  +-- MaintenanceSupervisor : Supervisor
        +-- checkpoint / compaction / spill jobs
```

An ordinary actor may spawn children, but `spawn` alone cannot guarantee
monitoring, restart intensity limits, epoch fencing or data recovery.
Supervision MUST use the common Atto supervisor/CONTROL semantics; avoid
introducing a new actor keyword. `extends Supervisor` is the intended typed
base, **pending verification on Java, LLVM, Wasm, Forge and .NET**.

A `KvDatabase` remains an immutable, **per-generation** routing snapshot:
`storage_epoch`, shard references and WAL reference must all describe one
atomic published topology. It must NOT be a process-wide, permanently cached
router across storage restarts. A new epoch gets new references; old snapshots
are rejected by the admission authority, not redirected to dead actors.

## Safety invariants

1. The process must acquire and hold one exclusive database directory lease
   before any store read/write, WAL writer or listener becomes active. An
   externally restarted process must not race an old process still writing.
2. The root supervisor owns the epoch/admission authority **outside** its
   restartable storage subtree. A crashed root cannot supervise itself:
   **fail-stop the process**, close/terminate old writers, then use a fresh
   exclusive lease and durable replay on external restart.
3. Storage progresses `fenced -> recovering -> ready`; no public commands
   are admitted in the other phases. An epoch is strictly monotonically
   increasing; overflow is fatal. No epoch is reused in the same process.
4. `StorageAdmission.enter(epoch)` must be an atomic, fail-closed,
   cancellation-safe, owned scope. Its guard survives every `await` through
   WAL append AND shard apply, and for GET through its full lookup. A
   supervisor's fence **closes admissions before draining** these scopes.
   Cancellation/restart must not release a guard while a WAL operation it
   protects is still running (including detached tasks).
5. Fencing, draining, and retiring the former topology happen before replay
   or new WAL writer construction. Do not restart just the WAL writer while
   old shards still have pending appends. Initial implementation uses a
   **one-for-all storage generation** restart, even for one failed shard.
6. A recovery proof must validate manifest, SST references and CRCs; replay
   valid ordered WAL records; reconcile monotonic LSN and tombstones; and
   restore **every** shard before the new WAL writer and routing snapshot can
   be published. Corrupt complete WAL frames or unsafe manifest/lease paths
   must fail closed, not be disguised as a healthy restart.
7. `publish_ready(epoch)` must atomically publish topology + epoch + serving
   state. An old `ready` completion or child CONTROL notice cannot reopen or
   fence a newer generation.
8. Root and storage supervisors receive child panic/termination on CONTROL.
   A panicking actor cannot catch its own panic and cannot change other
   actors' sealed state directly. `on_throw/on_raise` are not substitutes
   for parent supervision notices.
9. Only bounded transient failures are eligible for backoff/restart.
   WAL fsync failure, CRC/manifest corruption, failed store lease,
   unknown replay state and exhausted restart budget remain fenced for
   operator repair; never infinite-restart an unsafe store.
10. Network/client crashes use one-for-one bounded restart and must not
    reset durable storage. Reconnects/retries may duplicate non-idempotent
    client commands: never claim exactly-once delivery from supervisor restart.
11. Spill, checkpoint and compaction must participate in the same durable
    fence protocol. A checkpoint drains complete shard commands, fsyncs WAL,
    fsyncs SST + manifest + containing directory, then prunes **only through
    a globally durable fence**, never max(shard watermark). The existing
    `src/checkpoint.atto` remains a proposed API, not implemented behavior.
12. On shutdown: stop new network admission, fence and drain all commands,
    sync WAL according to policy, stop maintenance, then storage children.
    `everysec` acknowledges *acceptance*, not always durable persistence;
    do not promise crash persistence without the WAL durable acknowledgment.

## Failure response table

| Event | Parent response | Restart |
|---|---|---|
| Client actor failure | Close connection; bounded one-for-one replacement | Client only |
| TCP listener failure | Bound retries; no storage state mutation | Network subtree |
| Shard panic | Fence + drain old epoch, retire all storage children, WAL/manifest replay, publish fresh topology | Entire storage generation |
| WalWriter panic | Same; never resume against old shard state | Entire storage generation |
| StorageSupervisor panic | Root fences via CONTROL, retires subtree, replays durable state | New StorageSupervisor + generation |
| Compaction/checkpoint transient failure | Stop affected job, retry only after safe state inspection | Maintenance subtree |
| Checksum/manifest/fsync/store-lease integrity failure | Reject requests and retain diagnostic | **No automatic restart** |
| Restart budget exhausted | Stay fenced; escalate | **No automatic restart** |
| RedisRootSupervisor panic | Process fail-stop, external restart under exclusive lease | **New process only** |

## Current source and explicit missing implementations

- `src/redis_supervision.atto` declares the intended hierarchy. Its
  `Supervisor`, `SupervisorChildren`, `RootRecoveryLane`,
  `StorageAdmission`, `StorageRecovery`, `DatabaseLease` and
  `ProcessLifecycle` names are **proposed interfaces**. No Atto backend
  currently proves these declarations parse, typecheck or execute.
- `src/kv_database.atto` has three `StorageAdmission.enter` guards in
  SET/GET/DELETE and carries `storage_epoch`. This is a **target**,
  not verified real fencing, cancellation safety or actual lease ownership.
- `src/checkpoint.atto` still calls unimplemented durable cut/fence APIs.
  Integrate it with the root authority before claiming crash safety.
- `rust/tests/supervision_contract.rs` is a small deterministic Rust
  transition model. Its tests check phase and epoch invariants, restart budget,
  stale notices, duplicate completions, fatal corruption and fail-stop.
  **Passing these tests does not execute the Atto supervisors.**
- Rust storage and RESP tests remain separate. Do not equate a Rust green
  test suite with Atto backend or network supervision parity.

## Required next runtime gates

1. Implement a shared `Supervisor` base with parent/child monitoring, CONTROL
   panic routing, no self-panic interception, bounded restart windows,
   backoff and dead-parent propagation across each `oreslang-source.*`.
2. Implement atomic epoch admission and *whole async command* drain,
   including cancellation, watchdog preemption, old-child termination,
   and nonreentrant WAL/shard ordering. Prove no admitted old-generation WAL
   append can execute after new-generation replay begins.
3. Compile the real `src/redis_supervision.atto` and
   `src/kv_database.atto` against native Atto stdlib/storage APIs.
   Fix parser/type checker failures rather than claiming the Rust model
   proves compilation.
4. Execute fault injection: panic shard before/after WAL append/fsync/apply;
   kill WAL writer during fsync; kill storage supervisor during recovery;
   crash root; duplicate/late CONTROL; force epoch overflow and restart
   storms; poison manifest; cancellation during every `await`.
5. Assert every response and durability boundary after restart, durable
   snapshot/WAL hash, leaked task count, single writer, lock release and
   zero old-epoch acceptance. Repeat on Java/GraalVM, LLVM, .NET AOT,
   Forge and Wasm with backend-specific host storage capabilities.
6. Only after exact-source/compiler SHA, real guest exit code and runner
   receipts are collected may native recovery be marked `EXECUTED`.

Do not merge a proposed supervisor contract as a production-ready Atto Redis
server. Keep the stacked PR draft until native implementation and CI receipts.
