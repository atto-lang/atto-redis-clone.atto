# Atto Redis Clone

## Real Atto guest execution (not just Rust)

A compiler candidate actually executed source files from this repository: src/redis_kv_smoke.atto runs scalar-map SET/GET/DEL returning 42; src/redis_actor_guest.atto spawns a real Atto actor and emits set|get|delete. See [successful native Truffle guest CI](https://github.com/voxletra/voxletra-e2e/actions/runs/43039057038) and [Java compiler PR #2881](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2881).

The proposed full actor storage/checkpoint implementation remains NOT_COMPILED and native Atto TCP listener and WAL/SST interfaces remain NOT_IMPLEMENTED. There is no Atto-versus-Rust server throughput ratio. Exact source identities and missing gates are documented in docs/NATIVE_GUEST_EXECUTION.md.

This repository has **AttoLang actor source targets** and a **Rust-only executable reference**. Do not add Python implementations, Python tests or Python CI.

## Rust reference (std-only, Rust 1.85+, Unix)

    cargo test --manifest-path rust/Cargo.toml --offline
    cargo run --manifest-path rust/Cargo.toml --release --offline --bin atto-redis -- --data-dir ./var/atto-redis --host 127.0.0.1 --port 6379 --shards 64 --spill-mib 64 --fsync always --checkpoint-seconds 60

RESP2 subset: PING, ECHO, SET, GET, DEL, QUIT. Storage uses an ordered CRC32 WAL, hashmap shards, immutable disk segments, checkpointing and compaction. Single-node, loopback only, no auth, no TTL, no transactions or replication.

## Atto vs Rust benchmark

Run a compiled native Atto Redis server and the Rust server with the same data characteristics, shard count, sync policy, CPU allocation, process limits, memory headroom and cold/warm cache regime. Then use one **Rust** TCP workload driver:

    cargo run --manifest-path rust/Cargo.toml --release --offline --bin bench_resp -- --rust 127.0.0.1:6379 --atto 127.0.0.1:6380 --requests 100000

The driver performs 50% SET, 40% GET, 10% DEL (fixed 64-byte values, 1,024 key working set) with sequential request/reply. It reports each server's operations per second and an Atto/Rust ratio. This is intentionally a narrow workload: single connection, no pipelining, no multi-client scaling, fsync may dominate. Run randomized order and repeat trials for publishable comparisons.

**No compiled native Atto RESP server has been verified. Therefore Atto throughput and the ratio are NOT_EXECUTED, not zero, and not inferred from Rust.** Compilation of the Rust reference is not AttoLang compilation. Track the missing runtime and language primitives in docs/ACTOR_RUNTIME_GATES.md; src/*.atto remains a design target across the five backends.

### Layout

- src/kv_database.atto: sealed actor-owned shard target, WAL and command admission ordering
- src/checkpoint.atto: cross-shard durable manifest fence
- rust/src/lib.rs: Rust reference storage engine
- rust/src/server.rs: bounded RESP2 TCP handler
- rust/src/bin/bench_resp.rs: common RESP2 benchmark harness
- rust/tests/storage.rs: executable Rust correctness and protocol tests
- docs/ACTOR_RUNTIME_GATES.md: native execution blockers and acceptance tests

Do not expose the reference server to untrusted networks or claim production Redis correctness.


### Security and executable scope

The actual Java/GraalVM compiler has executed the Atto scalar Map guest and actor-owned SET/GET/DELETE smoke. Hardened compiler gates and legacy actor guest passed 25 tests: https://github.com/voxletra/voxletra-e2e/actions/runs/43041735134 . Java compiler PR: https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2881 .

The Rust RESP2 reference restricts commands to 32-byte names, bulk arguments to 4 MiB, aggregate request frames to 8 MiB, maximum 16 RESP arguments and 64 clients. Rust CLI and server library accept numeric **loopback IPs only**; a remote TCP listener without authentication/TLS is prohibited. This reference is NOT yet a native Atto Redis TCP server. For implemented-vs-proposed feature gates, see docs/ACTOR_RUNTIME_GATES.md and docs/NATIVE_GUEST_EXECUTION.md.

## Atomic missing-key guest milestone — 2026-11-11

Native AttoLang guest code now executes `Map<K,V>.get_optional(key): Option<V>` on the Java/GraalVM compiler candidate. The implementation performs one synchronized runtime lookup, never a racy separate contains+index read; the compiler rejects unsupported key/value domains and first-class extraction of that Map method. **This is native Atto guest execution, not a Rust surrogate.**

`src/redis_actor_guest.atto` now issues an initial GET on a missing key, SET, GET, DELETE and a second missing-key GET, returning exactly `nil|set|get|delete|nil`. The guest owns its in-memory scalar Map; it does **not** perform socket I/O or durable WAL work. Compiler candidate: [Java PR #2881](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2881) at `dd0fba9497b32d61c56bcc5231aa5684d6b575f0`; Redis actor source blob: `2b0d7e43fee4a06e6c54f7129501674fd11095a7`.

[Executed private exact-blob Java/GraalVM CI run #43055373575](https://github.com/voxletra/voxletra-e2e/actions/runs/43055373575) passed **27/27 real compiler/typechecker/Truffle guest JUnit tests**, with zero skipped. Carrier SHA: `ee4afb302b946cb38f1d528715b436caf3502a33`, tree `01e06c40e17574c7ba9bf5265434d48a6bf9e259`. The 362 pinned source manifest entries and four changed compiler/JUnit blobs were independently verified; this was an existing approved private mirror and involved no source-org Actions job admission.

**Not yet executed:** full `kv_database.atto` and `checkpoint.atto` compilation, native `std/net` TCP listener, RESP2 over Atto sockets, WAL/SST fsync and crash replay, AOT/multi-backend parity, and Atto/Rust throughput. The Redis PR and compiler PR must remain draft until actual executable gates pass.
