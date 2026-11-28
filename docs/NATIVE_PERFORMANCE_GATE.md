# Native Atto Redis ↔ Rust performance acceptance gate (v1)

**Status: source-authored test gate, NOT_EXECUTED on this PR head until a runner executes the Rust tests. No native Atto Redis server, 70% result, or source/compiler provenance is certified by this tool.**

The `parity_gate` Rust binary deliberately does not run servers or synthesize throughput. It validates two **separately measured** benchmark receipts, verifies configuration comparability, computes the median of at least five trial rates per side and fails if Atto achieves **less than 70%** of the Rust reference. It is a *second-stage arithmetic gate*, not the measurement harness, cryptographic auditor, or compiled guest verification. A malicious or mistaken receipt can still claim arbitrary values. Independently validate every source/compiler executable Git identity, executed CI step, workload and observed raw sample before treating its output as evidence.

## Pre-conditions

- Compile and execute the *actual* Atto RESP2 broker with one of Java/GraalVM, LLVM or .NET NativeAOT (not a scalar smoke or Java KV bridge). Native actor-owned map/WAL, physical per-actor 32-bit heap/stack, revocation/physical quiescence and nonblocking socket/fsync integration need independently executed tests.
- Compile the Rust RESP2 reference from a pinned revision. Both servers handle the **same full RESP2 command mix** over live TCP, and both pass reply, crash/restart, durability, and protocol correctness gates.
- Run on the same isolated machine, under identical CPU affinity, storage path/type, connection count, payload size, key distribution, warmup, operations, durability and ACK/flush policy. Use the same Rust workload driver and record raw per-trial results, p50/p95/p99, CPU/RSS and fsync counters. Do not compare ephemeral Atto against fsync-always Rust; don't use a preface-only or scalar guest for broker throughput.
- Require at least five warmup-excluded, unskipped, independently observed trials on each endpoint. Do not choose the faster of repeated experiments after seeing the results. Zero-step Actions results are runner admission failures, not execution evidence.

## Receipt format

The following is a **schema example with fake values**, not evidence. Exactly 17 keys are allowed. Supply one file for Atto and one for Rust; update backend (`java`, `llvm` or `dotnet` versus `rust`), `native_guest` (`true` versus `false`), and independently observed samples. Commit values must be **full 40-hex Git IDs**. Input format checks do not verify the claimed identities.

```ini
format=atto-redis-native-perf-v1
broker=redis
backend=llvm
workload=resp2_set50_get40_del10
scope=full_resp2_tcp_broker
durability=fsync_always
ack_policy=after_fsync
clients=8
payload_bytes=64
machine_id=isolated-runner-1
source_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
compiler_commit=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
executed_steps=12
correctness=PASS
native_guest=true
nonblocking_io=true
samples_ops_sec=70,70,70,70,70
```

```sh
cargo test --manifest-path rust/Cargo.toml --bin parity_gate --offline
cargo run --manifest-path rust/Cargo.toml --release --offline --bin parity_gate -- \
  --atto /path/to/pinned-atto-run.receipt --reference /path/to/pinned-rust-run.receipt
```

The tool emits `PERF_ARITHMETIC_PASS_NOT_CERTIFIED` only if the ratio is at least 0.70. It rejects missing or duplicate fields, zero executed steps, nonfinite/insufficient trials, unsupported backend substitution, mismatched workload/hardware/durability/ACK/scope, and non-native Atto claims. It cannot authenticate `machine_id`, Git SHA contents, native guest execution, nonblocking I/O, test completeness or benchmark fairness. Those are separate hard release gates.

This first implementation lives in Redis because it has the existing Rust RESP benchmark drivers. Equivalent protocol-specific contracts are needed for NATS Core/JetStream, Kafka partitions and RabbitMQ classic/quorum queues; do not reuse Redis operation mix to claim their performance parity.
