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
