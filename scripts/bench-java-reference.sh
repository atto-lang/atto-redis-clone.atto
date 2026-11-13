#!/usr/bin/env bash
# Independent Java RESP2 reference, validated by the same Rust checked workload.
# No Atto guest and no persistence in this reference. No Java/Rust speed ratio
# should be interpreted without normalizing durability.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
scratch="$(mktemp -d)"
pid=
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$scratch"
}
trap cleanup EXIT
mkdir -p "$scratch/classes"
javac --release 21 -d "$scratch/classes" \
  "$root/bridge/AttoRedisAsyncNioServer.java" \
  "$root/reference/java/RedisJavaReference.java" \
  "$root/reference/java/RedisJavaReferenceTest.java"
java -cp "$scratch/classes" RedisJavaReferenceTest
rustc --edition 2021 -O "$root/rust/src/bin/bench_resp_checked.rs" \
  -o "$scratch/bench_resp_checked"
java -cp "$scratch/classes" RedisJavaReference 0 >"$scratch/ready" 2>"$scratch/stderr" &
pid=$!
addr=
for _ in $(seq 1 100); do
  if ! kill -0 "$pid" 2>/dev/null; then
    cat "$scratch/stderr" >&2; exit 1
  fi
  addr="$(grep -Eo 'ADDR=/127[.]0[.]0[.]1:[0-9]+' "$scratch/ready" | tail -n 1 | sed 's#^ADDR=/##' || true)"
  if [[ -n "$addr" ]]; then break; fi
  sleep 0.1
done
if [[ -z "$addr" ]]; then
  echo "independent Java RESP2 reference never announced socket" >&2
  cat "$scratch/stderr" >&2
  exit 1
fi
"$scratch/bench_resp_checked" --java "$addr" --requests 500 --warmup 50 --trials 3
echo "REDIS_JAVA_REFERENCE_CHECKED=PASS DURABILITY=NONE ATTO_GUEST=NOT_EXECUTED"
