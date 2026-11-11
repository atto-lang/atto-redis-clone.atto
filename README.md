# Atto Redis Clone

This repository has **AttoLang actor source targets** and a **Rust-only executable reference**. Do not add Python implementations, Python tests or Python CI.

## Real Atto guest milestone (not a native Redis server)

Small reviewed `.atto` guest fixtures have now executed on actual compilers:

- Java/Truffle: one-key LSN/tombstone behavior and scoped async text filesystem round trip, **2/2 guest JUnit tests passed** ([CI](https://github.com/voxletra/voxletra-e2e/actions/runs/43043693979)).
- LLVM: a scalar LSN fixture compiled to LLVM IR, linked to a native executable and returned **42** ([CI](https://github.com/voxletra/voxletra-e2e/actions/runs/43044447246)).
- Rust infrastructure: **18/18 tests passed**, including 3 absence/tamper protections for the real-guest execution gate ([CI](https://github.com/voxletra/voxletra-e2e/actions/runs/43042975460)).

These results use pinned private compiler snapshots, not the current full compiler source heads; the full `src/*.atto` Redis server remains an uncompiled contract. Binary WAL fsync, native Atto RESP2 networking, Graal Native Image, and Atto/Rust performance are NOT_EXECUTED. Exact receipts and blockers: [native vertical-slice evidence](docs/NATIVE_VERTICAL_SLICE.md).

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


## Audit hardening and repeatable measurements

- Rust RESP2 parsing caps concurrent clients at 32, each request at 8 MiB and each bulk at 4 MiB; key/value limits are checked before allocation. Non-loopback listening is rejected unless the operator explicitly passes `--allow-remote` (unsafe without authentication and TLS).
- Rust WAL `everysec` mode is now independently synchronized by the background worker even after writes stop; `always` still synchronizes each acknowledged write. Both are different durability contracts.
- Rust recovery checks include complete corrupt WAL frames (fail closed), torn final WAL frames (truncate), exclusive database leases, concurrent writes, segment compaction, and real RESP2 binary frames.
- Atto source is still a non-compiled contract. GET participates in the serial admission lane, frozen generations stay retained until their durable segment is installed, and active-map accounting is adjusted after deletion and rotation.
- Run the identical `bench_resp` binary against both live native endpoints only. Until an Atto guest binary exists, Atto throughput and speed ratios remain **NOT_EXECUTED**. See [hardening and benchmark gates](docs/BENCHMARK_AND_HARDENING.md).

Do not expose the reference server to untrusted networks or claim production Redis correctness.
