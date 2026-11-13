#!/usr/bin/env bash
# Real compiler -> native scalar admission only. NEVER actor, I/O or broker proof.
set -Eeuo pipefail
if (( $# != 1 )) || [[ ! "$1" =~ ^(llvm|net|both)$ ]]; then
  echo 'usage: bash ci/native-llvm-net-scalar.sh llvm|net|both' >&2
  exit 64
fi
mode="$1"
root="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
for tool in git sha256sum timeout cmp; do
  command -v "$tool" >/dev/null || { echo "NOT_EXECUTED: missing $tool" >&2; exit 69; }
done
check_tree() {
  local label="$1" repo="$2" head="$3" tree="$4"
  [[ "$head" =~ ^[0-9a-f]{40}$ && "$tree" =~ ^[0-9a-f]{40}$ ]] || { echo "$label: invalid pin" >&2; exit 65; }
  [[ -d "$repo/.git" || -f "$repo/.git" ]] || { echo "$label: no checkout" >&2; exit 69; }
  [[ "$(git -C "$repo" rev-parse HEAD)" == "$head" ]] || { echo "$label: HEAD mismatch" >&2; exit 65; }
  [[ "$(git -C "$repo" rev-parse 'HEAD^{tree}')" == "$tree" ]] || { echo "$label: tree mismatch" >&2; exit 65; }
  [[ -z "$(git -C "$repo" status --porcelain --untracked-files=no)" ]] || { echo "$label: dirty tracked sources" >&2; exit 65; }
  printf '%s_SOURCE_HEAD=%s\n%s_SOURCE_TREE=%s\n' "$label" "$head" "$label" "$tree"
}
check_tree ATTO_APP "$root" "$ATTO_APP_EXPECTED_SHA" "$ATTO_APP_EXPECTED_TREE"
fixture="$root/tests/native/scalar_exact_source.atto"
[[ -f "$fixture" && ! -L "$fixture" && ! -L "$root/tests" && ! -L "$root/tests/native" ]] || { echo 'NOT_EXECUTED: missing/linked fixture' >&2; exit 66; }
blob="$(git -C "$root" hash-object -- "$fixture")"
[[ "$blob" == "$(git -C "$root" rev-parse 'HEAD:tests/native/scalar_exact_source.atto')" ]] || { echo 'Fixture bytes changed' >&2; exit 65; }
echo "ATTO_SCALAR_SOURCE_BLOB=$blob"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
run_native() {
  local label="$1" binary="$2" expected_status="$3" expected_output="$4" status=0
  [[ -f "$binary" && -x "$binary" && ! -L "$binary" ]] || { echo "$label: missing native executable" >&2; exit 69; }
  timeout --kill-after=2s 30s "$binary" >"$work/$label.stdout" 2>"$work/$label.stderr" || status=$?
  [[ "$status" -eq "$expected_status" ]] || { echo "$label: wrong exit $status (expected $expected_status)" >&2; cat "$work/$label.stderr" >&2; exit 1; }
  printf '%s' "$expected_output" >"$work/$label.expected"
  cmp "$work/$label.expected" "$work/$label.stdout" || { echo "$label: wrong output" >&2; exit 1; }
  [[ ! -s "$work/$label.stderr" ]] || { echo "$label: unexpected stderr" >&2; exit 1; }
  printf '%s_NATIVE_BINARY_SHA256=%s\n' "$label" "$(sha256sum "$binary" | cut -d' ' -f1)"
  printf '%s_EXECUTED=PASS\n' "$label"
}
if [[ "$mode" == llvm || "$mode" == both ]]; then
  llvm="$(cd -- "$ATTO_LLVM_CHECKOUT" && pwd -P)"
  check_tree LLVM "$llvm" "$ATTO_LLVM_EXPECTED_SHA" "$ATTO_LLVM_EXPECTED_TREE"
  # Build both tools from the exact checked-out tree; stale/foreign binaries
  # are not acceptable provenance for native source-to-AOT execution.
  for tool in clang rustc cmake llvm-config; do
    command -v "$tool" >/dev/null || { echo "LLVM: $tool missing" >&2; exit 69; }
  done
  cmake -S "$llvm" -B "$work/llvm-build" -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_TESTING=ON -DLLVM_DIR="$(llvm-config --cmakedir)" >"$work/llvm-configure.log"
  cmake --build "$work/llvm-build" --target oreslang-llvmc \
    oreslang-no-gc-ir-audit-build --parallel 2 >"$work/llvm-build.log"
  compiler="$work/llvm-build/oreslang-llvmc"
  auditor="$work/llvm-build/oreslang-no-gc-ir-audit"
  [[ -x "$compiler" && -x "$auditor" ]] || { echo 'Pinned-source LLVM compiler/auditor not built' >&2; exit 69; }
  echo "LLVM_COMPILER_BINARY_SHA256=$(sha256sum "$compiler" | cut -d' ' -f1)"
  echo "LLVM_AUDITOR_BINARY_SHA256=$(sha256sum "$auditor" | cut -d' ' -f1)"
  for level in 0 2; do
    ORESLANG_LLVMC="$compiler" ORES_NO_GC_IR_AUDIT="$auditor" \
    ORESLANG_AOT_LTO=off ORESLANG_LLVM_AOT_OPT_LEVEL="$level" \
      bash "$llvm/scripts/compile-aot.sh" "$fixture" "$work/llvm-O$level" >"$work/llvm-O$level.compile.log"
    run_native "LLVM_O$level" "$work/llvm-O$level" 42 ''
  done
fi
if [[ "$mode" == net || "$mode" == both ]]; then
  net="$(cd -- "$ATTO_NET_CHECKOUT" && pwd -P)"
  check_tree NET "$net" "$ATTO_NET_EXPECTED_SHA" "$ATTO_NET_EXPECTED_TREE"
  command -v dotnet >/dev/null || { echo '.NET SDK missing' >&2; exit 69; }
  (cd "$net" && bash scripts/publish-linux.sh "$fixture" artifacts/atto-broker-parity) >"$work/net-build.log"
  exe="$net/artifacts/atto-broker-parity/linux-x64/oreslang-app"
  (cd "$net" && bash verification/assert-linux-native-release.sh "$exe") >"$work/net-native-proof.log"
  run_native NET_NATIVE_AOT "$exe" 0 $'42\n'
fi
echo 'ATTO_NATIVE_SCALAR_ONLY=true'
echo 'ATTO_NATIVE_ACTOR_HEAP_CERTIFIED=false'
echo 'ATTO_NATIVE_SHARED_RW_LOANS_CERTIFIED=false'
echo 'ATTO_NATIVE_NONBLOCKING_BROKER_CERTIFIED=false'
echo 'ATTO_NATIVE_DURABILITY_CERTIFIED=false'
