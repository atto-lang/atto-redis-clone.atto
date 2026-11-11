# Native Atto guest execution gate

This repository requires **actual Atto compiler and guest execution**, not merely Rust reference checks.

The limited `src/redis_map_guest.atto` is a supported candidate using a typed nonempty Map and returns 42. It tests source-to-guest execution on Java/GraalVM but is **not** a Redis server or native storage interface implementation.

The full `src/kv_database.atto` and `src/checkpoint.atto` must additionally compile, link and run. Their `SerialCommandLane`, `WalStorage`, `SegmentStore`, `WalAck`, and sealed actor-state dependencies have no verified linked implementation yet. Rejecting them with explicit parser/type errors is evidence of a blocker, **not** executing the Redis server.

Cross-organization source-transfer controls: run against already existing and verified private compiler snapshots. Never copy source to a public repository or treat a source-free test as guest execution. Collect compiler snapshot SHA, full Redis file blob SHAs, GitHub run/job IDs, actual tests and diagnostics.

Before any comparison with Rust, execute a native Atto RESP server over sockets using the same commands, client count, 64-byte values, workload seeds and `fsync=always`. No speed ratio exists before native Atto server execution.


## Actual Atto guest execution evidence — 2026-11-11

All programs listed below were submitted to the real Java/GraalVM Atto compiler (OresCompiler.parseAndTypeCheck) and, where accepted, executed with Truffle Context.eval, not the Rust reference.

- src/redis_map_guest.atto: typed Map literal and indexed read, returns 42. Exact Git blob 5cefcd66c2d45471c5c7897a238567db1baadcd1.
- src/redis_kv_smoke.atto: mutable typed Map<String,int> performs set("answer",41), get, delete (true) and validates empty, returning 42. Git blob 5637be64ab7e5bfebcdef8db2f91d8ebfd67fac3. Executed run: https://github.com/voxletra/voxletra-e2e/actions/runs/43037860295
- src/redis_actor_guest.atto: a real spawned Atto KvShardSmoke actor owns a scalar map and handles message commands set/get/delete. Blob e73b4c5847ac134e9d7695436016a4a7315d9ce3. Passed 7 actual compiler-backed JUnit tests: https://github.com/voxletra/voxletra-e2e/actions/runs/43039057038. Witness output: set|get|delete.
- Java source compiler snapshot 053b76b96ab65e560c21efbaa609fc8584e27868, with exactly two modified Java compiler blobs: TypeChecker 77c635b988acbc5c7d5deb2b9a738e0ac72316db and Truffle OresEvalRootNode e5b1ae4378a3d96772cc2909f86f29b342cd5ff3.
- Proposed Java compiler integration: https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2881. This is a stacked draft not merged into Java mainline.
- KvShardSmoke uses legacy actor fields, not verified sealed handle_state loans.

### Nonnegotiable native server gaps

The full src/kv_database.atto was rejected by the actual Java parser at 7:16 (class field initializer requires : or =), while src/checkpoint.atto was rejected at 6:32 (expected opening brace). More critically, WalStorage, SerialCommandLane, SegmentStore, WAL ack/fsync, SST checkpoint and related source runtime interfaces do not have linked guest implementations.

The Java backend document NATIVE_NETWORK_IO_AND_TRACING.md explicitly classifies source-level std/net nonblocking TCP listen/accept/read/write as NOT YET IMPLEMENTED. The existing JDK HTTP adapter is not a raw TCP RESP listener written in Atto.

Therefore a complete in-Atto Redis RESP server, persisted Atto WAL and SST execution, and an identical-condition Atto-vs-Rust throughput comparison remain NOT_EXECUTED / NOT_MEASURED. Do not substitute this actor smoke for the server.


### Subsequent compiler and Rust hardening — 2026-11-11

Java compiler PR #2881 added strict scalar Map domain checks, direct-call-only mutator gates, writable local/actor owner admission and compiler rejects for captured mutator values and const writes. Run: https://github.com/voxletra/voxletra-e2e/actions/runs/43041735134 (25/25 compiler-backed JUnit cases). Hardened TypeChecker Git blob 672360990f0d57ac0cbbb847f47087d5e69625e4. This does not certify all mutable alias / borrow / cross-actor transport graphs.

Rust server security adds 4 MiB/bulk and 8 MiB/request preallocation checks, 32-byte command cap, panic-safe connection permits and numeric loopback-only bind through both CLI and library. This is reference security work; NOT an executable in-Atto RESP service. Current formatter check reports preexisting Rust formatting drift and is nonblocking.

## Atomic missing-key guest milestone — 2026-11-11

Native AttoLang guest code now executes `Map<K,V>.get_optional(key): Option<V>` on the Java/GraalVM compiler candidate. The implementation performs one synchronized runtime lookup, never a racy separate contains+index read; the compiler rejects unsupported key/value domains and first-class extraction of that Map method. **This is native Atto guest execution, not a Rust surrogate.**

`src/redis_actor_guest.atto` now issues an initial GET on a missing key, SET, GET, DELETE and a second missing-key GET, returning exactly `nil|set|get|delete|nil`. The guest owns its in-memory scalar Map; it does **not** perform socket I/O or durable WAL work. Compiler candidate: [Java PR #2881](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2881) at `dd0fba9497b32d61c56bcc5231aa5684d6b575f0`; Redis actor source blob: `2b0d7e43fee4a06e6c54f7129501674fd11095a7`.

[Executed private exact-blob Java/GraalVM CI run #43055373575](https://github.com/voxletra/voxletra-e2e/actions/runs/43055373575) passed **27/27 real compiler/typechecker/Truffle guest JUnit tests**, with zero skipped. Carrier SHA: `ee4afb302b946cb38f1d528715b436caf3502a33`, tree `01e06c40e17574c7ba9bf5265434d48a6bf9e259`. The 362 pinned source manifest entries and four changed compiler/JUnit blobs were independently verified; this was an existing approved private mirror and involved no source-org Actions job admission.

**Not yet executed:** full `kv_database.atto` and `checkpoint.atto` compilation, native `std/net` TCP listener, RESP2 over Atto sockets, WAL/SST fsync and crash replay, AOT/multi-backend parity, and Atto/Rust throughput. The Redis PR and compiler PR must remain draft until actual executable gates pass.
