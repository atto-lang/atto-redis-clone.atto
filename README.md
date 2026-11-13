# Atto Redis Clone

## Native AttoLang RESP2 server, 2026-11-13 (current)

`src/server.atto` is a Redis-compatible server written entirely in AttoLang: RESP2 parsing, dispatch and replies, with one store task owning a `Map<Bytes, Bytes>` and a session task per connection. It passes **231/231** exact-byte conformance checks (`rust/src/bin/resp_conformance.rs`), the same checks real `redis-server` passes. Throughput on the Java backend is **8–11% of redis-server** without pipelining and about **1–2%** with pipelining, so the 70% target is **not met**. LLVM and .NET cannot compile it yet. Commands, numbers and causes: [docs/NATIVE_ATTO_SERVER_2026_11_13.md](docs/NATIVE_ATTO_SERVER_2026_11_13.md).

Run: `JAVA_HOME=<JDK 25 + Graal> java ... dev.oreslang.launcher.OresMain --allow=NETWORK,ACTOR_SPAWN src/server.atto` (listens on 127.0.0.1:6380).

The older sections below describe JDK-bridged experiments. Their RESP parsing ran in host Java, so their throughput figures are **not** AttoLang performance.

## Executed Atto actor RESP2 benchmark — diagnostic, not native parity

The real AttoLang actor executes SET/GET/DEL behind a loopback JDK21 TCP/IPC
bridge, and the Rust-written client verifies every RESP reply. An expanded
10-trial private exact-blob run [43094013417](https://github.com/voxletra/voxletra-e2e/actions/runs/43094013417)
passed **14/14 benchmark tests**, **4/4 Atto compiler/Truffle guest tests**,
live Atto RESP2 TCP, and verified cleanup after every trial.

- Atto actor + bridge: **4,393.9 median ops/sec**
- Rust durable WAL fsync=always: **7,071.1 median ops/sec**
- Diagnostic ratio: **62.14%**, **not apples-to-apples** because only Rust is durable.

A separate exact-source rerun with **8/8 real Atto guest tests** passed and measured
**6,494.7 Atto actor ops/sec** versus **6,367.3 Rust ops/sec** (diagnostic ratio
1.0200), showing substantial inter-run variability. Do not infer native
Atto/Rust speed parity or statistical superiority from these numbers.

Each of ten trials had 1,000 warm-up, 2,000 measured and 758 correctness-checked
out-of-timer key deletions on each endpoint. See [execution and protocol
limits](docs/ATTO_ACTOR_PERFORMANCE.md). The native Atto async TCP and WAL
implementation remains unexecuted. **Do not treat these numbers as native Atto
versus Rust language-performance parity.**


## M1.5 PROVED: real Atto actor owns Redis KV over TCP

A spawned **AttoLang actor** now owns the in-memory `Map<string,string>`,
accepts mailbox SET/GET/DEL operations and returns results through its Atto
output stream. The JDK-only RESP2 transport forwards the binary-safe protocol,
without implementing KV storage. This is an executable actor guest, not just
a proposed architecture. The [funded exact-blob integration
run #43063897766](https://github.com/voxletra/voxletra-e2e/actions/runs/43063897766)
passed 4 real Java/Truffle guest tests (0 failures/skips) and a pipelined
binary RESP2 TCP test against that actual actor implementation.

Use [actor guest launch instructions](docs/ACTOR_KV_M1_5.md):
`bash scripts/run-atto-redis-actor-loopback.sh /absolute/compiler-checkout 6380`.

**Development-only and ephemeral**: this actor variant uses legacy direct
mutable actor fields (not sealed state loans); Atto-native TCP,
WAL/fsync/recovery, durable storage and native Atto/Rust benchmarking remain
unimplemented. Do not expose this unauthenticated loopback server publicly.

## M1 IMPLEMENTED: actual AttoLang Redis-like TCP execution (ephemeral)

A real `.atto` guest now owns in-memory SET/GET/DEL state and handles
binary-safe Redis RESP2 requests through a **loopback-only JDK21 transport
bridge**. The host only parses/fragments TCP and forwards encoded bytes;
it does not implement or substitute the guest's KV storage.
PING/ECHO/QUIT are protocol-layer transport commands.

**Executed evidence:** [funded exact-blob Java/Truffle and TCP RESP2 carrier
run #43062003166](https://github.com/voxletra/voxletra-e2e/actions/runs/43062003166)
passed 3 real guest JUnit tests with zero skips, plus a real TCP client
pipelining PING, binary SET/GET/DEL, absent key, empty live value, and QUIT.
The required compiler fix is
[Java PR #2883](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2883).

See [working local launch instructions and limitations](docs/LOOPBACK_EPHEMERAL_M1.md).
This new service is **EPHEMERAL, JDK-bridged and NOT production Redis**:
it has NO WAL/fsync/restart durability, no Atto-native listener, no auth/TLS,
no supervised actor storage, and no valid native Atto-vs-Rust benchmark.
The Rust reference remains the only durable RESP/WAL implementation.

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

## Additional Rust durability hardening (experimental)

The Rust reference synchronizes WAL `everysec` in the background even when writes
stop. `always` fsyncs before a successful acknowledgment; `everysec` can lose
recently accepted writes after power failure. Storage resource bounds and
crash-recovery tests are tracked in [the benchmark and hardening guide](docs/BENCHMARK_AND_HARDENING.md).

**Security:** The unauthenticated reference only binds numeric loopback IPs.
No `--allow-remote` bypass is provided. This remains an experimental Redis
subset, not a verified native Atto server or production-ready network service.

## Native Atto guest vertical slice (still not a Redis server)

`guest/kv_cell.atto` and `guest/fs_roundtrip.atto` have been executed by
a pinned Java/Truffle compiler snapshot; `guest/llvm_lsn_smoke.atto` has
been compiled to a native LLVM executable with exit 42. These are focused
guest milestones (one-key mutable cell, scoped text filesystem, scalar LSN),
not a working durable Atto RESP listener. See
[exact guest evidence and compiler limitations](docs/NATIVE_VERTICAL_SLICE.md).
The Rust-only `verify_guest` gate fails closed when native compilers or
verified source blobs are missing.

## Supervised storage generation (proposed Atto runtime APIs)

The Redis supervisor [contract](docs/SUPERVISION_AND_RECOVERY.md) adds a
`RedisRootSupervisor` and storage, network and maintenance supervisors.
Storage restart fences and drains a complete WAL/shard generation before
durable replay. Old `KvDatabase` routing snapshots carry a generation
epoch and must reject stale requests, including direct actor references.
Root failure requires **process fail-stop**. Corrupt manifests/CRC and
exhausted restart budgets stay fenced.

`src/redis_supervision.atto` and its `StorageAdmission` APIs are
**NOT_COMPILED** proposals. The 13 Rust supervision-model tests exercise
transition logic but are **not** Atto runtime or crash/fault-injection proof.
Do not claim native Redis actor/server readiness until the toolchains
compile and execute the full storage + network APIs.
