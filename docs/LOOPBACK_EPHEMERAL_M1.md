# Atto Redis M1: executable loopback RESP2 development server

## Milestone and evidence

This branch implements a **working hybrid AttoLang Redis-like development server**.
A trusted Java 21 transport process listens only on 127.0.0.1:6380.
A child AttoLang program really runs under Java/Truffle and owns the KV Map:
all SET, GET and DEL operations execute in the .atto guest, not Java or Rust.
PING, ECHO and QUIT are protocol-level replies from the Java bridge.

Exact-byte private carrier CI run 43062003166 (2026-11-11) **SUCCEEDED**:
https://github.com/voxletra/voxletra-e2e/actions/runs/43062003166
- Real Atto guest JUnit: 3 tests, 0 failures, 0 errors, 0 skipped.
- Real TCP RESP2 pipeline against the actual Atto child: PASSED.
- Binary NUL and non-UTF-8 key/value bytes, empty live value, missing GET,
  SET/GET/DEL, PING and QUIT: verified at the TCP boundary.
- Compiler fix source: oreslang-source.java PR #2883, stacked on #2881.
- Carrier SHA: f9bd91959947920b040b606246457c87bb11b11f
- Atto guest Git blob: 8267b132222c7ce60a355174972483681cf8561f
- Compiler TypeChecker patched blob: 207e8ee706d137187e874006e0abe4bc07e73bfd

## What is implemented

- guest/redis_stdio.atto: persistent single-process Atto Map<string,string>
  storing hex-encoded, owned byte strings. Option-based GET distinguishes
  absent key and an empty present value.
- bridge/AttoRedisRespBridge.java: bounded TCP RESP2 transport, hex
  serialization to child, no host-owned KV store and no fallback.
- bridge/AttoRedisGuestMain.java: stdin-wired polyglot Context using
  allowAllAccess(false) and actual source typechecking.
- scripts/run-atto-redis-loopback.sh: a one-command developer launcher
  that verifies the exact guest Git blob before running.

The generic OresMain compiler CLI did not forward stdin to the guest.
The dedicated restricted context launcher fixes this; the first carrier
attempt failed on actual TCP because the guest received EOF, and the second
carrier executed the corrected launcher and passed end to end.

## Build and run locally

Requirements: JDK 21+, Maven, git, redis-cli and an authorized Java compiler
checkout at feat/atto-redis-stdin-option-v1-20261111 (PR #2883,
stacked on Java compiler PR #2881). Build the compiler checkout first:

    mvn -B -ntp '-P!native-thread-unix' -DskipTests package
    mvn -B -ntp '-P!native-thread-unix' dependency:build-classpath -Dmdep.outputFile=target/conformance-classpath.txt

Then from this Redis implementation branch:

    bash scripts/run-atto-redis-loopback.sh /absolute/path/to/oreslang-source.java 6380

In another terminal:

    redis-cli -h 127.0.0.1 -p 6380 PING
    redis-cli -h 127.0.0.1 -p 6380 SET sample hello
    redis-cli -h 127.0.0.1 -p 6380 GET sample
    redis-cli -h 127.0.0.1 -p 6380 DEL sample

The bridge will refuse to listen until the real Atto child emits READY.
Startup failure, premature child exit, IPC timeout or unexpected replies fail
closed, never silently run a Java or Rust KV fallback.

## Strict safety and feature limitations

This is **ephemeral**. Every shutdown loses the KV state. It is NOT an Atto
native TCP listener, not an actor-owned sealed shard, and not a complete Redis
implementation. Binary WAL/fsync, SST, manifest, checkpoints, recovery, TTL,
transactions, authentication, replication, cluster mode and supervisor restart
are NOT IMPLEMENTED / NOT EXECUTED in this runnable M1 service.

The bridge uses Java TCP and a child process with hex-encoded IPC and
serial command execution; therefore **do not label it native Atto throughput
or compare it as native Atto vs optimized Rust**.

The listener is hard-coded to loopback. The transport has no authentication
or TLS. Do not expose it to untrusted networks or public internet.

Bounds include: 32 client workers, a bounded 32-item queue, 10s client idle
timeout, 1MiB per bulk, 4KiB key, max three arguments, 20s startup timeout,
15s guest IPC timeout. Memory quotas and process isolation still require
audit before any production exposure.

## Production finish line

M2: implement a real Atto native async TCP / binary RESP2 frontend and an
actor-owned sealed KV shard with correct across-await ordering.

M3: implement Atto-native WAL, durability, crash replay, SST checkpoint,
epoch-fenced supervisor recovery and validate via fault injection.

M4: compile same Atto guest on LLVM and .NET/Forge/Wasm (where relevant),
then benchmark comparable network/durability paths versus Rust with exact
compiler and source SHAs, RSS and p50/p95/p99 latency.

Do not merge this milestone as a production-durable Redis replacement.
