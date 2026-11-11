# Atto Redis M1.5: genuine actor-owned KV at real TCP boundary

## Verified execution (2026-11-11)

The actor variant has **executed successfully**, not merely compiled.
The Redis Map is stored inside a real spawned Atto actor and SET / GET / DEL
operations execute on its mailbox handler; the outer Atto command loop sends
serially to this actor and awaits its output stream. A tiny JDK21 transport
shim provides binary-safe RESP2 TCP on loopback only.

[Exact-blob private funded carrier run 43063897766](https://github.com/voxletra/voxletra-e2e/actions/runs/43063897766),
job 130673556857, carrier head a239d8d9eee30763b0eda574ba704114f4cd05c8:

- AttoRedisActorPersistentGuestTest: **1/1** real compiled actor guest, zero failures/errors/skips.
- Other Option/Map/persistent Atto guest tests: **3/3** passed.
- Total: **4/4** actual compiler-backed JUnit guest tests passed.
- Real TCP RESP2 integrated Atto actor-child test: PASS.
- Executed log witnesses:
  ATTO_REDIS_ACTOR_KV_REAL_GUEST=EXECUTED_PASS
  ATTO_REDIS_REAL_ACTOR_TCP=EXECUTED_PASS
  ATTO_REDIS_BINARY_KEY_EMPTY_VALUE_PIPELINE=PASS
- Canonical actor fixture blob c777f0ca9de51febacbf91d350e329c53614b059.
- Fixed compiler Option return blob 207e8ee706d137187e874006e0abe4bc07e73bfd.
- JDK21 operates in interpreter fallback for current Truffle; JDK25 optimizing
  runtime and Graal Native Image are NOT_EXECUTED.

## Run locally

Use the Java compiler checkout from PR #2883 (stacked on #2881), built as
described in docs/LOOPBACK_EPHEMERAL_M1.md. Run this Redis branch:

    bash scripts/run-atto-redis-actor-loopback.sh /absolute/oreslang-source.java 6380

Then:

    redis-cli -h 127.0.0.1 -p 6380 PING
    redis-cli -h 127.0.0.1 -p 6380 SET sample hello
    redis-cli -h 127.0.0.1 -p 6380 GET sample
    redis-cli -h 127.0.0.1 -p 6380 DEL sample

These are actual Atto actor operations, not Rust or Java Map proxies.
The exact source is guest/redis_actor_stdio.atto.

## Important caveats

This is a single actor with a legacy direct mutable Map field, not yet
the language's new sealed state container or a compiler-verified borrow-checked
critical state closure. The transport serializes messages to one logical
writer, and the actor currently uses a small multi-message command state
machine (SET opcode, key and value). It is not a general concurrent cluster.
The source requires further hardening for actor failure, cancellation,
generation fencing, mailbox limits, hot upgrade and state loan rules.

It is still **ephemeral**. On process restart the Map is empty. Binary
WAL/fsync, SST and checkpoint, supervisor recovery, durable disk storage,
authentication, TLS, replication and Atto-native TCP are not implemented in
this executable M1.5 path. Do not expose to public internet or market as
production Redis. Do not compare its throughput against native Rust and
call that a fair native Atto/Rust comparison; this path includes JDK IPC.

The Rust reference remains the persistent and benchmarkable baseline.
The next production milestone is a durable Atto-owned storage actor
integrated with the runtime's explicit WAL/file capabilities.
