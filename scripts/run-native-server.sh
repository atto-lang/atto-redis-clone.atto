#!/usr/bin/env bash
# Run the native AttoLang RESP2 server (src/server.atto) on the Java/GraalVM
# backend with the Truffle optimizing runtime and HotSpot JIT settings used
# for the recorded measurements.
#
# Usage: scripts/run-native-server.sh /abs/path/oreslang-source.java
#   ATTO_JDK      JDK whose JVMCI matches the compiler's Truffle version
#                 (Truffle 25.3.4.1 needs Labs OpenJDK jvmci-25.3-b22).
#   ATTO_GRAAL_UP colon-separated Graal 25.3.4.1 jars for --upgrade-module-path
#                 (compiler, options, collections, word, truffle-compiler).
# Without ATTO_GRAAL_UP the server still runs, but Truffle falls back to its
# interpreter and the numbers are not comparable.
set -euo pipefail
compiler="${1:?usage: run-native-server.sh /abs/path/oreslang-source.java}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
java_home="${ATTO_JDK:?set ATTO_JDK to a JDK 25 with JVMCI matching Truffle}"
classpath="$(cd "$compiler" && mvn -B -q -o dependency:build-classpath -Dmdep.outputFile=/dev/stdout)"
jit=()
if [[ -n "${ATTO_GRAAL_UP:-}" ]]; then
  jit=(-XX:+UnlockExperimentalVMOptions -XX:+EnableJVMCI "--upgrade-module-path=$ATTO_GRAAL_UP")
fi
tmp="$(mktemp -d)"; tmp="$(cd "$tmp" && pwd -P)"
# The evaluator's dispatch methods exceed HotSpot's 8000-bytecode JIT limit.
exec "$java_home/bin/java" "${jit[@]}" -XX:-DontCompileHugeMethods \
  -Djava.io.tmpdir="$tmp" \
  -Dores.thread.native.path="$compiler/target/native/liboresthread.dylib" \
  --enable-native-access=ALL-UNNAMED \
  -cp "$compiler/target/classes:$classpath" dev.oreslang.launcher.OresMain \
  --allow=NETWORK,ACTOR_SPAWN "$repo/src/server.atto"
