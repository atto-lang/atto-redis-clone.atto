# Atto Redis: borrow-checked shared payloads and mailbox capabilities

Status: **design gate**, not an implemented SharedSlice allocator. The executing 8/32-client guest fixtures remain on the safe, owned `Bytes` + finite actor `List<Tuple[... ]>` path. See Redis PR #37, Java issue #1932.

## Executable baseline and invariants

1. Each accepted NIO2 `NioConnection` is an affine, **exclusive** capability moved once into its owner session Task. A socket is *not* an immutable blob, even if an address to its wrapper is const. Only its owner may accept/read/write/close; its `NioListener` remains on the accept task.
2. Each session reads **at most 16 fully parsed** binary RESP2 command frames, then moves the owned `List<Tuple[int,Bytes,Bytes]>` through a finite typed Channel. Channel contents are **not** borrowed stack slices. Java's existing `TypeAutoTraits` marks `Bytes` Send/Share, `NioConnection` Send-only and `List<T>` Send-only, while `OwnershipChecker` rejects borrowed Channel payloads and use-after-move.
3. Exactly one Atto KV actor owns `Map<Bytes,Bytes>`. Each accepted batch is executed serially in one actor turn and emits one tagged ordered reply list; exactly one broker reads the actor output. Every session verifies its batch sequence ID and response count before emitting wire bytes. Saturated queues must produce backpressure, never unbounded staging.
4. `ActorRuntime.freeze` retains proven immutable `OresBytes` identity in SHARED actor transport while creating an immutable container graph. The PRIVATE/UNTRUSTED `isolateCopy` route physically clones each Bytes payload. `OresBytes.copyOf` clones untrusted buffers on ingress and `copyToArray` clones on extraction. This is **not** end-to-end zero-copy socket I/O: framing and outbound NIO serialization still detach byte arrays, and command container metadata is allocated.
5. Messages cannot carry a raw native pointer, JNI/OS handle, mutable `ByteBuffer`, active `MutexGuard`, unfenced mutable borrow, or unverified foreign object. Never infer permission to alias an accepted TCP socket from `SHARED_MEMORY` being enabled.

## Proposed optional large-payload optimization (not active)

Use a trusted runtime-only sealed allocation registry. A source-visible `SharedSlice<Bytes>` is an **opaque, unforgeable read-only capability**; the mailbox carries a *lease token*, not a raw pointer. The runtime checks:

| Field | Purpose |
|---|---|
| allocation_domain / region_id | Trusted region registry lookup |
| generation | ABA protection on region reuse |
| offset / length | Bounds checked with overflow-safe arithmetic against sealed allocation |
| rights | Read-only; never mutable across actor domains |
| owner_epoch | Fences actor restart and stale callbacks |
| lease_id | Exactly-once escrow, cancellation and release accounting |
| destination_incarnation | Binds the authorized receiver, not just actor name |

The region must be fully initialized and sealed **before** publishing the token to the mailbox. No user code may construct the capability from integers or turn it into an unrestricted `Pointer<T>`. Cross-actor code may read only within a compile-checked, non-suspending readonly borrow scope. Persistent write access requires exclusive ownership transfer, never a shared lease.

### Admission / cleanup protocol

- **Before admission:** validate actor incarnation, type, rights, byte length, mailbox slots, outstanding waiters, per-recipient lease count and global pinned bytes; escrow ownership and quota exactly once.
- **At linearization point:** commit the mailbox receipt and only then publish the receiver's lease; reject/cancel before commit rolls back escrow, but never creates a second socket/region owner.
- **After commit:** destination owns cleanup, even if sender dies. A cancellation/QUIT/STOP/PANIC or actor restart cannot free a backing region while an I/O completion, borrow or continuation still uses it. Release only after last lease plus physical quiescence.
- **Forked readers:** many readonly leases may point at one sealed region, with bounded per-recipient reference quotas and global pinned-byte quotas. A mutator must use an exclusive, separate region/epoch; seal-then-mutate is rejected.
- **Transport choice:** keep current `Bytes` move path for small messages and all PRIVATE/UNTRUSTED boundaries unless the new capability is proven. Evaluate sealed slices for larger values (e.g. >=4 KiB) *only after* measured A/B proves net savings over registry lookup, pinning and lease costs.
- **Cross-backend:** LLVM must prove native arena ownership + generation atomics and sanitizers; Java must validate interop isolation; WebAssembly Workers and .NET AOT must implement equivalent opaque handles or fail closed and copy. Never send a raw process pointer across isolates.

## Required CI before enabling SharedSlice in production

1. Compile-time reject `writech requests, rt borrow payload`, use after `rt take`, mutable borrow across `await`, duplicate socket transfer, direct buffer/FD channel payloads.
2. Negative runtime tests: forged region, stale generation, offset overflow, rights escalation, seal-then-mutate, double release, actor death with pinned borrows, STOP/CANCEL/PANIC while the bounded USER queue is saturated and generation ABA on slot recycling.
3. Verify zero copied value bytes **and** quota receipts by instrumentation rather than trusting pointer identity alone. Distinguish retained immutable bytes from truly zero-copy NIO receive/write.
4. Paired exact-SHA real Atto 1/8/32/100-client A/B against current batched-owned Bytes, official ephemeral Redis and a durability-matched variant; record p50/p95/p99, allocation bytes/op, pinned bytes, GC and fairness. No feature promotion from a single-client microbenchmark.

The immediately actionable code path is **actor-turn batching with owned immutable Bytes**. The shared-lease allocator and user-level pointer transport remain deliberately disabled until the security/conformance gates are implemented.
