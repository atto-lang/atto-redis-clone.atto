# Atto Redis Clone

A Redis-like key/value database experiment targeting AttoLang actor isolation, bounded memory and deterministic ownership. This repository was bootstrapped on 2026-11-11.

**Evidence boundary:** The Python implementation under \`reference/\` is an executable **storage and protocol oracle**. The \`src/*.atto\` source is a **native AttoLang implementation target**, not currently verified against the AttoLang compiler or any of the five backends. GitHub Actions in this PR exercise Python only and must NOT be reported as native AttoLang guest execution.

## What is implemented in the runnable reference

- Stable CRC32 key partitioning into configurable shards (64 by default), independently guarded maps and bounded pending generations.
- RESP2 over TCP: \`PING\`, \`ECHO\`, \`SET\`, \`GET\`, \`DEL\`, \`QUIT\` (not full Redis compatibility).
- CRC-checked binary write-ahead log with monotonically increasing LSNs; \`fsync=always\` acknowledges after file sync; \`everysec\` is a deliberate durability trade-off.
- Automatic immutable memtable rotation when a shard exceeds its configured byte budget, a background disk writer, sorted immutable SSTable segments and newest-wins lookups.
- Atomic, fsynced segment manifest; checkpoint barriers, WAL pruning after persisted checkpoint, segment compaction and restart recovery.
- Crash-tail truncation of incomplete WAL frames; complete checksum failures fail closed instead of silently discarding acknowledged data.
- Tests covering data integrity, binary values, recovery, checkpointing, compaction, concurrency, corruption and real TCP protocol round-trips.

## Run the reference server (Python 3.11+)

\`\`\`sh
PYTHONPATH=reference python -m atto_redis_reference.server \
  --host 127.0.0.1 --port 6379 --data-dir ./var/atto-redis \
  --shards 64 --spill-mib 64 --fsync always
\`\`\`

Use \`redis-cli -h 127.0.0.1 -p 6379 SET hello world\` and \`redis-cli GET hello\`, or use any RESP2 client issuing the supported commands.

\`\`\`sh
PYTHONPATH=reference python -m unittest discover -s reference/tests -v
PYTHONPATH=reference python -m compileall -q reference
\`\`\`

The hot-map limit counts approximate Python allocation sizes, **not** actual RSS; SST indexes remain resident in RAM. This is a functional prototype, not a hard-memory-limit system. \`fsync=always\` has a platform/filesystem-dependent durability envelope; power-loss correctness requires platform testing.

## Layout

- \`src/kv_database.atto\` — actor-owned KV shards, WAL writer and router target
- \`src/checkpoint.atto\` — snapshot and spill orchestration target
- \`reference/atto_redis_reference/engine.py\` — executable sharded WAL/SST engine
- \`reference/atto_redis_reference/server.py\` — executable RESP2 server
- \`reference/tests/\` — correctness and recovery probes
- \`docs/ACTOR_RUNTIME_GATES.md\` — compiler/runtime contracts needed to replace Python with native AttoLang

## Not implemented

Authentication, TLS, ACLs, TTL/expiry, transactions, Lua, replication, clustering, eviction, Redis persistence format compatibility, memory-mapped indexes, sparse indexes and multikey atomicity. There are no native AttoLang compiler conformance results. Do not expose the reference server on an untrusted network.
