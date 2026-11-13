#!/usr/bin/env bash
# Local M1.5: REAL Atto actor-owned KV guest executes SET/GET/DEL; JDK21 provides RESP2 TCP transport.
# This is ephemeral and intentionally loopback only, not the durable native server.
set -euo pipefail
# This historical subprocess/stdio bridge performs BLOCKING socket and pipe
# I/O. It is NOT an async Atto server and MUST NEVER be a default run path.
# Opt-in is allowed only for loopback-only diagnostic compatibility tests.
if [[ "${ATTO_ALLOW_BLOCKING_LEGACY_DIAGNOSTIC:-}" != "I_ACCEPT_LOCAL_BLOCKING_IO" ]]; then
  echo "BLOCKED: legacy synchronous TCP/stdio bridge; use a native Atto async NIO guest instead." >&2
  echo "For isolated loopback diagnostics ONLY, set ATTO_ALLOW_BLOCKING_LEGACY_DIAGNOSTIC=I_ACCEPT_LOCAL_BLOCKING_IO." >&2
  exit 77
fi
if (( $# < 1 || $# > 2 )); then
  echo "usage: $0 /absolute/oreslang-source.java-checkout [PORT(default=6380)]" >&2
  exit 64
fi
root="$(cd "$(dirname "$0")/.." && pwd -P)"
compiler="$(cd "$1" && pwd -P)"
port=6380
if (( $# == 2 )); then port="$2"; fi
if [[ ! "$port" =~ ^[0-9]+$ ]] || (( 10#$port < 1 || 10#$port > 65535 )); then
  echo "invalid TCP port" >&2
  exit 64
fi
if [[ ! -d "$compiler/target/classes" || ! -s "$compiler/target/conformance-classpath.txt" ]]; then
  echo "NOT_EXECUTED: build the compiler and its Maven dependency classpath first" >&2
  exit 69
fi
guest="$root/guest/redis_actor_stdio.atto"
if [[ ! -f "$guest" || -L "$guest" ]]; then
  echo "NOT_EXECUTED: canonical Atto guest must be a regular non-symlink file" >&2
  exit 66
fi
actual="$(git hash-object "$guest")"
if [[ "$actual" != c777f0ca9de51febacbf91d350e329c53614b059 ]]; then
  echo "NOT_EXECUTED: .atto guest has drifted from the reviewed blob SHA" >&2
  exit 65
fi
command -v javac >/dev/null || { echo "JDK21 javac required" >&2; exit 69; }
command -v java >/dev/null || { echo "JDK21 java required" >&2; exit 69; }
builddir="$(mktemp -d)"
trap 'rm -rf "$builddir"' EXIT
classpath="$compiler/target/classes:$(cat "$compiler/target/conformance-classpath.txt")"
javac --release 21 -cp "$classpath" -d "$builddir" \
  "$root/bridge/AttoRedisRespBridge.java" "$root/bridge/AttoRedisGuestMain.java"
java -cp "$builddir" AttoRedisRespBridge --port "$port" -- \
  java -cp "$builddir:$classpath" AttoRedisGuestMain "$guest"
