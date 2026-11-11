# Atto Redis — first real guest execution gate (2026-11-11)

This directory contains **actual AttoLang guest source**, not a Rust/Java emulation.
The source is an *incremental compilation and runtime milestone*, not a working
Redis server. In particular, `guest/kv_cell.atto` stores only **one key** and
uses String rather than a binary-safe Bytes type. It exercises monotonic LSN
application, stale ack rejection, tombstones and empty live values.

## Reviewed fixtures

| Guest file | Git blob SHA | Meaning |
|---|---|---|
| `guest/kv_cell.atto` | `6a075fc0f0f36a4bb04cebc98dabfb183898f43f` | Mutable guest class: one-key SET/GET/DELETE state transitions |
| `guest/fs_roundtrip.atto` | `fb1c2fba1c0ec48681b2c086f41ec2d4f0b37e31` | Java guest async text write/read/remove template |
| `guest/llvm_lsn_smoke.atto` | `d20a410d4ce9bd27626b675437535e808692f0f5` | Portable scalar LSN next-step proof (exit 42) |

The expected outputs are tracked in `guest/*.expected`; a missing output
file is **not** treated as an empty success. The Rust gate rejects source drift
by computing the real Git blob SHA, and rejects missing compilers with
`NOT_EXECUTED` (exit 2).

The Java/GraalVM integration lives in
[oreslang-source.java PR #2882](https://github.com/ores-truffle-oreslang/oreslang-source.java/pull/2882).
It mirrors the **identical** checked-in source Git blobs, then uses
`LinkedProgramRunner.validate` and `LinkedProgramRunner.run` for real guest
execution. It tests `fs_roundtrip.atto` only with a substituted scoped temp path
and explicit READ/WRITE permissions. Full JDK 25/Maven execution must be
performed and logged; until then all new guest tests are `NOT_EXECUTED`.

### Commands on an authorized, pinned Java checkout

~~~sh
# In pinned oreslang-source.java checkout: this runs the real guest JUnit.
mvn -B -ntp -Dtest=AttoRedisGuestVerticalSliceTest test
# In atto-redis-clone.atto checkout: after Java compiler build, the pinned
# Java launcher from PR #2882 accepts the in-memory fixture as its only arg.
cargo run --offline --manifest-path rust/Cargo.toml --bin verify_guest -- \
  --mode java --source guest/kv_cell.atto \
  --source-blob 6a075fc0f0f36a4bb04cebc98dabfb183898f43f \
  --expected-output guest/kv_cell.expected \
  --runner /absolute/java-checkout/scripts/run-atto-redis-guest.sh
~~~

This runner requires an absolute pre-reviewed script; it is not a source-free
acceptance test. The JUnit FS test uses the language runtime's deny-all
permission context, not an unrestricted CLI default. Do not run the FS template
with an arbitrary path or broad filesystem grants.

### Commands on an authorized, pinned LLVM checkout

~~~sh
# Build/verify compiler first; it must accept .atto for this fixture.
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=ON
cmake --build build --parallel 2
# Invoke from the atto-redis repo root:
cargo run --offline --manifest-path rust/Cargo.toml --bin verify_guest -- \
  --mode llvm --source guest/llvm_lsn_smoke.atto \
  --source-blob d20a410d4ce9bd27626b675437535e808692f0f5 \
  --expected-exit 42 \
  --runner /absolute/llvm-checkout/scripts/compile-aot.sh
~~~

An LLVM scalar exit-42 guest run does **not** certify actors, network I/O,
filesystem durability, or cross-backend Redis parity. The LLVM compiler may
still reject native .atto until source-extension admission is implemented.

## Native blocking requirements

1. **Binary WAL**: length-checked CRC framing, append, monotonic LSN assignment,
   explicit sync completion, fail-closed error outcomes and crash restart.
   Java `fs.write_text` / `fs.append_text` are not substitutes for fsync.
2. **Actor semantics**: actual `define serial actor`, sealed state and
   `SerialCommandLane` ownership through `await`, with GET/SET/DEL/rotation
   linearizability and deterministic cancellation.
3. **Storage**: bounded native Bytes/HashMap, frozen memtable retention, SST
   indexing/compaction, manifest fsync and safe WAL pruning.
4. **Networking**: an actual Atto RESP2 TCP server, with key/value limits,
   backpressure, connection counts and loopback-only admission.
5. **Parity**: run the exact same positive and negative .atto guest sources
   independently on Java, LLVM, .NET Native AOT, Forge and Wasm where host
   capabilities exist. An unsupported backend remains NOT_EXECUTED or FAIL,
   not a synthetic PASS.
6. **Performance**: benchmark only real native RESP endpoints against the
   pre-existing Rust RESP2 driver, with equal fsync policies, client counts,
   key cardinalities, disk and CPU and at least five repetitions.

Evidence must identify source Git tree, compiler Git tree, versions, runner
and workflow/run/job IDs, exit status, expected/observed bytes and guest
invocation. Rust host-only checks must never certify Atto code.
