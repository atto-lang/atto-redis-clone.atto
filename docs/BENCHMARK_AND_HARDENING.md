# Redis clone hardening and Atto versus Rust benchmark protocol

## Evidence boundary

Rust has the executable storage engine and RESP2 frontend. AttoLang files under src/ remain a non-compiled design target. Python correctness results and Rust measurements are not Atto measurements. Never report an Atto/Rust speed ratio without two executed native programs and exact compiler and source SHAs.

## Correctness gates

1. Verify SET/GET/DEL, empty and binary values, restart and checkpoint/compaction visibility.
2. Reject complete bad CRC frames; truncate only incomplete final WAL frames; reject path traversal and symlink tricks in manifests.
3. Prove a second process cannot open the store, corrupted SSTs fail closed, and WAL ack policy matches fsync behavior.
4. Prove linearizable same-key SET/DELETE/GET histories. Serial Atto actor processing is insufficient unless turns preserve FIFO order across WAL awaits. The proposed ShardCommandGate must cover all commands through disk acknowledgment and map application. Never borrow sealed state across await.
5. Frozen memtables remain in pending until that exact SST generation is durably published. Rotate must clear active_bytes; delete must recompute it.
6. Checkpoint order: fence writers, flush immutable generations, fsync SSTs, atomically write manifest, fsync directory, then prune WAL. A manifest publish must not race spill or compaction.
7. After WAL success followed by in-memory allocation failure, stop/poison the shard and report ambiguous outcome, rather than silently losing a write.
8. Network: bind loopback, no authentication, no TLS. Rust RESP2 parser limits to 8 MiB/request, 4 MiB/bulk, 32 connections. Rust storage values up to 64 MiB and keys up to 1 MiB. Unauthenticated remote binding is forbidden, including via the library API.

## Fair performance comparison protocol

Run native compiled Atto guest and optimized Rust on the same dedicated CPU, OS/kernel, disk/filesystem, thread count, and data durability mode. Record CPU model, RAM, rustc and Atto compiler versions and exact SHAs, build settings, commands, RSS, and p50/p95/p99 latencies.

Workloads: SET, GET, mixed 50/50, 128-byte values. Pure writes have distinct keys; reads/mixed share 1024 seeded keys; use identical key strings and request patterns. Benchmark both as in-process API calls or both as RESP2 over TCP; never compare different layers. Test 1 and 8 threads, fsync=always and fsync=everysec separately, at least 5 runs each, median ops/s and latency, CPU/RSS/disk bytes/fsync count. Extend to 1k/100k/1m keys, 16/128/4096-byte values, skew and background SST compaction.

The reference's pending memtables and SST indexes consume resident memory. The configured active hashmap budget is not an overall RSS limit. A performance run does not prove crash safety; repeat crash/recovery and corruption checks on both platforms.

## Rust-only baseline commands

~~~sh
cargo test --manifest-path rust/Cargo.toml --all-targets
cargo build --release --manifest-path rust/Cargo.toml --bin atto-redis --bin bench_resp
# Start Rust reference on loopback in a separate terminal (see README).
rust/target/release/bench_resp --rust 127.0.0.1:6379 --requests 10000
~~~

The benchmark measures in-process Rust only. Atto native throughput remains NOT_EXECUTED until real backend guests compile and run. If source-org Actions runs fail before steps, use an exact-tree, per-blob-SHA-verified private mirror and distinguish carrier execution from original-source CI.
