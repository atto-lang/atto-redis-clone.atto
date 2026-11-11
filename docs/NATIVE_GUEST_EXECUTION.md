# Native Atto guest execution gate

This repository requires **actual Atto compiler and guest execution**, not merely Rust reference checks.

The limited `src/redis_map_guest.atto` is a supported candidate using a typed nonempty Map and returns 42. It tests source-to-guest execution on Java/GraalVM but is **not** a Redis server or native storage interface implementation.

The full `src/kv_database.atto` and `src/checkpoint.atto` must additionally compile, link and run. Their `SerialCommandLane`, `WalStorage`, `SegmentStore`, `WalAck`, and sealed actor-state dependencies have no verified linked implementation yet. Rejecting them with explicit parser/type errors is evidence of a blocker, **not** executing the Redis server.

Cross-organization source-transfer controls: run against already existing and verified private compiler snapshots. Never copy source to a public repository or treat a source-free test as guest execution. Collect compiler snapshot SHA, full Redis file blob SHAs, GitHub run/job IDs, actual tests and diagnostics.

Before any comparison with Rust, execute a native Atto RESP server over sockets using the same commands, client count, 64-byte values, workload seeds and `fsync=always`. No speed ratio exists before native Atto server execution.
