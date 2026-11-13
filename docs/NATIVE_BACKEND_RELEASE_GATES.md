# Native Atto broker: actor memory, borrow and I/O release gates

Status: **not production certified**. The current branch improves fail-closed
entrypoints and adds one canonical cross-backend **logical state** guest;
it does NOT establish full native broker parity or actor-heap isolation.

## Mandatory memory ownership

Every actor has a unique, incarnation-fenced persistent heap/arena and
short-lived per-turn scratch allocations. A supervisor thread pool is not an
actor's own heap, scheduler or task pool. Stopping an actor must revoke
admission, wait for suspended continuations, async I/O callback retirement
and outstanding memory loans, then scrub/free actor-local allocations exactly
once. A newly spawned actor with the same numeric ID cannot address the old
arena. A garbage-collected host object shared between JVM actors is NOT
evidence of physically separate private heaps.

Across a mailbox, admit only a provable copy, a move of exclusive ownership,
an immutable shared value or a checked shared-region capability.
**Never transport a naked pointer, movable buffer reference, NIO socket,
actor-state reference or global shared mutable map.**

For optional zero-copy shared regions, the compiler/runtime must issue
unforgeable bounded (region identity, generation, offset, length, rights,
owner incarnation) capabilities. A reader loan allows concurrent readers;
a writer loan requires **zero concurrent readers and exactly one writer**.
Admission is atomic with mailbox transfer and quota accounting. A writer
must release its exclusive loan before await, cancellation, actor shutdown or
cross-domain transfer. A read loan across suspension is legal only with a
runtime-enforced lifetime pin and no mutable alias. Region destruction waits
for physical retirement of all loans, including cancelled operations.
Illegal double-writes, stale generations, dangling read loans, alias escapes
and mutations through read-only handles MUST reject at compile-time or
runtime capability admission, never silently copy an unsafe pointer.

## Mandatory zero-blocking I/O

Actor execution carriers NEVER call blocking sockets, sync file/console
reads, Java Future.get/join, Thread.sleep, synchronous .NET Stream.Read,
or synchronous fsync. Do not hide blocking I/O in Task.Run, a dedicated
thread pool or a Java bridge and call it nonblocking. Network readiness must
use real kernel completion/readiness (for example epoll or OS async sockets)
with bounded buffers, deadlines, cancellation, single-owner close and
physical callback retirement. Disk durability MUST use a real async disk
completion path or remain unsupported; no fake ACK before durable commit.
CONTROL mail must remain independent from congested DATA mail. No actor
state read/write critical loan survives an I/O await.

Historical local scripts guarded by ATTO_ALLOW_BLOCKING_LEGACY_DIAGNOSTIC
are **not** certified Atto paths even when the diagnostic explicitly opts in.

## Three-backend execution: no substitute successes

The identical `tests/native_actor_isolation_guest.atto` source exists in
all four broker repositories. `ci/run-three-backend-actor-guest.sh`
requires the exact application head/tree and guest Git blob; independently
pinned Java/Graal, LLVM and .NET compiler heads/trees; and SHA256-pinned,
checked-out compiler-backed launchers. In addition, each compiler checkout
must be its own Git worktree root (never the application or a reused backend
checkout), contain the corresponding Java/LLVM/.NET source manifest in pinned
HEAD, and provide a reviewed executable launcher whose exact blob is tracked
in that HEAD. A generated/untracked marker-echoing wrapper cannot qualify.
Three independent adversarial denials run in source-only CI via
`ci/test-three-backend-admission-negative.sh`; passing those negatives is
NOT evidence that any native guest actually executed. No backend may be skipped. A
backend-specific launcher must **compile and execute** the source; using the
Java VM to simulate LLVM/.NET, a host-side Rust/Java actor, or a marker-echoing
script is explicitly invalid. An independent review of each launcher,
compiler-produced binary, target ABI and execution trace is mandatory.

This one fixture checks **two independent logical sealed-state actor
counters only**. It cannot prove native arena separation by itself. Require
backend-native instrumentation that asserts actor-local persistent/scratch
allocation identity, stale-handle rejection, ownership transfer,
reader-reader success, reader-writer and writer-writer exclusion, cancel/
close/ABA behavior, and arena scrub/physical retirement with ASan/UBSan
where applicable. Require a separate async TCP and durable I/O matrix:
partial reads/writes, EAGAIN rearm, out-of-order completion, EOF/RST,
timeouts, cancellation at each transition, stale completion, and continued
progress of independent actor carriers while I/O is pending.

## Current backend capability boundary (not an execution claim)

- **Java/Graal:** legacy guest actor and NIO2 test slices have executed
  in separately pinned runners. Sealed namespace actor source, full
  per-actor private-heap isolation and the whole native broker still
  require exact-branch verification.
- **LLVM:** native AOT scalar and restricted actor-mail slices exist;
  full native actor-state/Bytes/shared-borrow lowering and guest async
  network/durable I/O are not yet production certified. Host epoll tests
  are not the same as an executing Atto guest.
- **.NET NativeAOT:** actor-private unmanaged Arena runtime substrate and
  scalar NativeAOT code generation exist; general source-level actors,
  borrow-checked RW shared regions and guest async/await TCP/FS lowering
  remain fail-closed. No CoreCLR fallback qualifies as NativeAOT.
- No default workflow in these application repos can claim all three
  backends passed merely because the host reference or static source
  checks succeeded.

## Application-specific broker gate

Redis: one KV actor (or explicit shard owners) must own Map/LSN/WAL state; async NIO2 client sessions require affine socket ownership, generation-fenced bounded mailbox replies, pipeline order, RESP2 bytes, actor retirement and durable WAL/fsync. 100-session source work on draft #39 is NOT a mainline native/LLVM/.NET result. Legacy stdio/Java TCP bridges remain local opt-in diagnostics only.

## Evidence labels

Record the exact source SHA/tree, identical guest blob, compiler SHA/tree,
runner SHA256, actual process/binary identity, executed test count,
negative tests and full CI job steps. A queued/skipped/zero-step action,
source-only lint, host reference, fake compiler or modified fixture is
**NOT_EXECUTED**. Keep the backend gate red until all executable checks
pass. Require independent security and performance review before any merge
or public listener deployment.
