#!/usr/bin/env bash
# Real-source, fail-closed three-backend Atto guest admission.
# This script NEVER substitutes a host-only test/double or skips .NET/LLVM.
# Launchers MUST compile and execute the supplied unchanged Atto source.
# A green result proves two independent actor-local *logical* counters only;
# physical arena segregation, shared-region RW borrow safety, async TCP
# and NativeAOT require separate runtime/memory/IO conformance receipts.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
guest="$root/tests/native_actor_isolation_guest.atto"

need_sha() {
  if [[ ! "$2" =~ ^[0-9a-f]{40}$ ]]; then
    echo "INVALID_IDENTITY: $1 must be a lowercase 40-hex Git SHA" >&2
    exit 65
  fi
}
need_sha256() {
  if [[ ! "$2" =~ ^[0-9a-f]{64}$ ]]; then
    echo "INVALID_IDENTITY: $1 must be a lowercase 64-hex SHA256" >&2
    exit 65
  fi
}
: "${ATTO_APP_SHA:?exact application commit SHA required}"
: "${ATTO_APP_TREE:?exact application tree SHA required}"
: "${ATTO_GUEST_BLOB:?exact shared guest Git blob SHA required}"
need_sha ATTO_APP_SHA "$ATTO_APP_SHA"
need_sha ATTO_APP_TREE "$ATTO_APP_TREE"
need_sha ATTO_GUEST_BLOB "$ATTO_GUEST_BLOB"
[[ -f "$guest" && ! -L "$guest" ]] || { echo "UNSAFE_GUEST_PATH" >&2; exit 65; }
test "$(git -C "$root" rev-parse HEAD)" = "$ATTO_APP_SHA"
test "$(git -C "$root" rev-parse 'HEAD^{tree}')" = "$ATTO_APP_TREE"
test "$(git -C "$root" hash-object "$guest")" = "$ATTO_GUEST_BLOB"
git -C "$root" diff --quiet HEAD -- tests/native_actor_isolation_guest.atto

command -v timeout >/dev/null || { echo "MISSING: real watchdog timeout" >&2; exit 69; }
command -v sha256sum >/dev/null || { echo "MISSING: sha256sum" >&2; exit 69; }
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
# An application checkout, nested subdirectory or reused compiler checkout
# must never impersonate an independent native backend.
declare -a accepted_compiler_roots=()

run_backend() {
  local backend="$1" home_var="ATTO_${1}_COMPILER_CHECKOUT"
  local sha_var="ATTO_${1}_COMPILER_SHA" tree_var="ATTO_${1}_COMPILER_TREE"
  local runner_var="ATTO_${1}_RUNNER" digest_var="ATTO_${1}_RUNNER_SHA256"
  local checkout="${!home_var:-}" sha="${!sha_var:-}" tree="${!tree_var:-}"
  local runner="${!runner_var:-}" digest="${!digest_var:-}"
  [[ -n "$checkout" && -n "$runner" ]] || {
    echo "${backend}=NOT_EXECUTED: missing pinned compiler checkout or real runner" >&2
    return 69
  }
  need_sha "$sha_var" "$sha"
  need_sha "$tree_var" "$tree"
  need_sha256 "$digest_var" "$digest"
  [[ "$checkout" == /* && -d "$checkout" && ! -L "$checkout" ]] || {
    echo "${backend}=NOT_EXECUTED: invalid compiler checkout" >&2; return 69;
  }
  local compiler
  compiler="$(cd "$checkout" && pwd -P)"
  local git_root expected_manifest
  git_root="$(git -C "$compiler" rev-parse --show-toplevel 2>/dev/null)" || {
    echo "${backend}=DENIED: not a standalone Git compiler checkout" >&2; return 65;
  }
  [[ "$git_root" == "$compiler" && "$compiler" != "$root" ]] || {
    echo "${backend}=DENIED: nested or application checkout is not an independent compiler" >&2
    return 65
  }
  for prior in "${accepted_compiler_roots[@]}"; do
    [[ "$compiler" != "$prior" ]] || {
      echo "${backend}=DENIED: reused compiler checkout across backend roles" >&2
      return 65
    }
  done
  case "$backend" in
    JAVA) expected_manifest="pom.xml" ;;
    LLVM) expected_manifest="CMakeLists.txt" ;;
    NET) expected_manifest="compiler/Ores.Net.Compiler/Ores.Net.Compiler.csproj" ;;
    *) echo "DENIED: unknown compiler role" >&2; return 65 ;;
  esac
  # Require a compiler-family source manifest in the pinned Git tree, not an
  # untracked stand-in copied into an arbitrary app checkout.
  git -C "$compiler" cat-file -e "HEAD:$expected_manifest" 2>/dev/null || {
    echo "${backend}=DENIED: expected compiler manifest absent from pinned tree" >&2
    return 65
  }
  test "$(git -C "$compiler" rev-parse HEAD)" = "$sha"
  test "$(git -C "$compiler" rev-parse 'HEAD^{tree}')" = "$tree"
  git -C "$compiler" diff --quiet HEAD --
  [[ "$runner" == /* && -f "$runner" && -x "$runner" && ! -L "$runner" ]] || {
    echo "${backend}=NOT_EXECUTED: missing executable native/compiler launcher" >&2
    return 69
  }
  local runner_dir actual_runner
  runner_dir="$(cd "$(dirname "$runner")" && pwd -P)"
  actual_runner="$runner_dir/$(basename "$runner")"
  [[ "$actual_runner" == "$compiler/"* ]] || {
    echo "${backend}=NOT_EXECUTED: launcher must live inside pinned checkout" >&2
    return 65
  }
  # An untracked generated wrapper can print the expected marker without
  # compiling anything. The launcher entrypoint must exist in pinned HEAD.
  # Its reviewed implementation may build a native binary in the CI workspace.
  local relative_runner="${actual_runner#"$compiler"/}"
  git -C "$compiler" cat-file -e "HEAD:$relative_runner" 2>/dev/null || {
    echo "${backend}=DENIED: launcher is not tracked in pinned compiler tree" >&2
    return 65
  }
  test "$(git -C "$compiler" rev-parse "HEAD:$relative_runner")" = "$(git -C "$compiler" hash-object "$actual_runner")" || {
    echo "${backend}=DENIED: launcher differs from pinned Git blob" >&2
    return 65
  }
  test "$(sha256sum "$actual_runner" | cut -d' ' -f1)" = "$digest"
  accepted_compiler_roots+=("$compiler")
  echo "${backend}_COMPILER_SHA=$sha"
  echo "${backend}_COMPILER_TREE=$tree"
  echo "${backend}_LAUNCHER_SHA256=$digest"
  # Each runner is independently reviewed to COMPILE and execute its source,
  # not to echo a marker, run an old artifact, interpret another guest or
  # launch the Java backend in place of native LLVM/.NET NativeAOT.
  if ! timeout --kill-after=10s 300s "$actual_runner" "$guest" \
      > "$tmp/$backend.stdout" 2> "$tmp/$backend.stderr"; then
    cat "$tmp/$backend.stderr" >&2
    echo "${backend}=NOT_EXECUTED_OR_FAILED" >&2
    return 1
  fi
  if [[ -s "$tmp/$backend.stderr" ]] || \
     ! diff -u <(printf '%s\n' 'ATTO_TWO_ACTOR_LOGICAL_ISOLATION=PASS') "$tmp/$backend.stdout"; then
    cat "$tmp/$backend.stderr" >&2
    echo "${backend}=EXECUTED_OUTPUT_MISMATCH" >&2
    return 1
  fi
  echo "${backend}_REAL_ATTO_GUEST=EXECUTED_PASS"
}

echo "ATTO_APP_SHA=$ATTO_APP_SHA"
echo "ATTO_APP_TREE=$ATTO_APP_TREE"
echo "ATTO_GUEST_BLOB=$ATTO_GUEST_BLOB"
run_backend JAVA
run_backend LLVM
run_backend NET
echo "THREE_BACKEND_LOGICAL_ACTOR_GUEST=EXECUTED_PASS"
echo "PHYSICAL_ACTOR_HEAP_PROOF=NOT_ESTABLISHED_BY_THIS_TEST"
echo "NONBLOCKING_IO_PARITY=NOT_ESTABLISHED_BY_THIS_TEST"
