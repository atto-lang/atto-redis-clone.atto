# Redis — exact-source LLVM and .NET NativeAOT admission

This repository has NOT certified a complete native Redis broker. Tracking issue:
https://github.com/ores-truffle-oreslang/atto-redis-clone.atto/issues/40

The same byte-identical tests/native/scalar_exact_source.atto is checked into
all four brokers. It proves only real Atto frontend -> native artifact scalar
execution, NEVER an actor heap, borrow, asynchronous I/O or broker protocol.

## Execute on an authorized Linux x86_64 carrier

Use privately authorized local LLVM and .NET source checkouts with known,
reviewed 40-hex commit and Git tree identities. Build real LLVM compiler plus
no-GC IR auditor using CMake. Install LLVM/Clang, .NET 10, NativeAOT prerequisites.

    ATTO_APP_EXPECTED_SHA=<app-commit> \
    ATTO_APP_EXPECTED_TREE=<app-tree> \
    ATTO_LLVM_CHECKOUT=<approved-absolute-llvm-checkout> \
    ATTO_LLVM_EXPECTED_SHA=<llvm-commit> \
    ATTO_LLVM_EXPECTED_TREE=<llvm-tree> \
    ATTO_NET_CHECKOUT=<approved-absolute-net-checkout> \
    ATTO_NET_EXPECTED_SHA=<net-commit> \
    ATTO_NET_EXPECTED_TREE=<net-tree> \
      bash ci/native-llvm-net-scalar.sh both

LLVM runs real Clang-linked O0 and O2 binaries, both exit 42 with empty
stdout/stderr. .NET publishes real NativeAOT compiler and app, verifies
ELF/native release layout without managed sidecars, then requires exact
"42" newline with zero exit. Record actual binary SHA-256. Missing compiler,
dirty tracked source, wrong HEAD/tree, symlink, timeout, unexpected output,
non-native fallback or missing result is failure. Focused 'llvm'/'net' mode
may be used while debugging, but BOTH are needed for scalar two-backend parity.

## Strict production rules — independent gates, not certified here

- Every actor incarnation owns a bounded private heap/arena and logical
  continuation frames. No raw pointer, OS descriptor or mutable CLR/JVM
  object is passed to another actor. Validate foreign/stale handles, quota,
  cancellation, overflow sibling survival and zeroizing physical retire.
- Typed bounded mailboxes and explicit ownership move (rt take) by default.
  Shared immutable memory gets generation-fenced read leases. Mutating
  shared memory needs exclusive writer admission: readers>0 implies
  writers=0; writer=1 implies readers=0. Cancel/STOP/restart revoke leases
  and do not park any actor/CONTROL carrier. Mutable loans cannot cross await.
- All guest-reachable TCP and durable storage operations must use TRUE
  nonblocking kernel readiness/completion, not blocking I/O wrapped in a
  Future, Task.Run, worker, or thread pool. A dedicated bounded reactor may
  poll the kernel, but must not execute guest logic. Unsupported platforms
  and fsync modes fail closed instead of fake async certification.
- Await only inside declared suspends fnc/mtd; bound every mailbox, socket
  buffer, in-flight write, WAL ticket and timer; fence late completions.
- Actual native Redis certification requires the SAME REVIEWED .atto
  broker guest compiled AND EXECUTED with per-actor memory and kernel I/O
  under LLVM AOT and .NET NativeAOT, independent wire clients, fault/recovery
  tests, borrow rejection and exact-run receipts. Scalar and host-only results
  can never be substituted for this.

Preflight CI only tests shell syntax, Git fixture identity and rejection of
missing source pins. Native compiler and broker execution remain NOT_EXECUTED
unless actual end-to-end receipts include exact commit/tree, fixture blobs,
native binary hashes, nonzero GitHub Actions job steps, tests, and classifications.
An org with zero runner steps has not run tests. An approved, private,
byte-identical cross-org tree mirror may be used without cross-org tokens.

## Additional native source-actor admission

A pinned LLVM checkout now runs `tests/source_actor_e2e.py` through its rebuilt
`oreslang-llvmc` and Clang: at least 16 compiled native source-actor
positive cases, plus the suite's no-IR negative ownership/await cases.
The script requires its explicit completion receipts. This exercises
**LLVM's bounded i64 actor prototype**, not any broker guest, Bytes/Map,
shared exclusive-writer loan implementation, or production I/O.

After publishing the real .NET NativeAOT compiler and scalar guest, the gate
also runs the compiler itself against the pinned `net_actor_reject.atto`
fixture twice: `check` and `compile`. Both must fail with diagnostics,
no stdout, and no generated assembly. This is *negative admission*, not a
claim that .NET actor lowering works. The .NET compiler must gain an
independently verified actor-native lane before this check is replaced.

The CI workflow is **source-only** and cannot emit these native receipts.
Successful native receipts require a properly provisioned Linux runner,
pinned private LLVM and .NET checkouts, and execution of
`bash ci/native-llvm-net-scalar.sh both`.

If a source organization has zero-step runner admission failures, an
**approved private** funded test repository can receive an exact, reviewed
commit-tree mirror using the GitHub connector, with source and destination
Git tree/blob identities verified before executing. Never put private
compiler sources into a public repository, embed a cross-org checkout
token, or accept an approximate copy or host-only result as native proof.
