#!/usr/bin/env bash
# Source/host-only adversarial admission tests. Never executes a native Atto
# compiler, and never claims actor heap or nonblocking I/O conformance.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
app_sha="$(git -C "$root" rev-parse HEAD)"
app_tree="$(git -C "$root" rev-parse 'HEAD^{tree}')"
guest_blob="$(git -C "$root" hash-object "$root/tests/native_actor_isolation_guest.atto")"

# A marker-echoing wrapper is adversarial; deny it before it is executed.
cat > "$tmp/fake-runner" <<EOF
#!/usr/bin/env bash
touch "$tmp/executed"
printf '%s\n' 'ATTO_TWO_ACTOR_LOGICAL_ISOLATION=PASS'
EOF
chmod +x "$tmp/fake-runner"
run_negative() {
  local label="$1" checkout="$2" runner="$3" expected="$4"
  local sha tree digest rc
  sha="$(git -C "$checkout" rev-parse HEAD)"
  tree="$(git -C "$checkout" rev-parse 'HEAD^{tree}')"
  digest="$(sha256sum "$runner" | cut -d' ' -f1)"
  rm -f "$tmp/executed"
  set +e
  ATTO_APP_SHA="$app_sha" ATTO_APP_TREE="$app_tree" ATTO_GUEST_BLOB="$guest_blob" \
  ATTO_JAVA_COMPILER_CHECKOUT="$checkout" \
  ATTO_JAVA_COMPILER_SHA="$sha" ATTO_JAVA_COMPILER_TREE="$tree" \
  ATTO_JAVA_RUNNER="$runner" ATTO_JAVA_RUNNER_SHA256="$digest" \
    bash "$root/ci/run-three-backend-actor-guest.sh" > "$tmp/out" 2> "$tmp/err"
  rc=$?
  set -e
  if [[ "$rc" -ne 65 ]] || ! grep -Fq "$expected" "$tmp/err" || [[ -e "$tmp/executed" ]]; then
    echo "ADMISSION_NEGATIVE_FAIL: $label (rc=$rc)" >&2
    cat "$tmp/out" "$tmp/err" >&2
    exit 1
  fi
}

# 1. The application checkout must never impersonate the Java compiler.
run_negative same-application "$root" "$tmp/fake-runner" \
  "nested or application checkout is not an independent compiler"

# 2. An independent Git repository without the Java compiler's manifest is
# not accepted merely because its HEAD/tree and launcher digests are pinned.
mkdir "$tmp/standin"
git -C "$tmp/standin" init -q
printf 'not a compiler\n' > "$tmp/standin/README.md"
git -C "$tmp/standin" add README.md
git -C "$tmp/standin" -c user.email=test@example.invalid -c user.name=GateTest commit -qm standin
run_negative unrelated-git "$tmp/standin" "$tmp/fake-runner" \
  "expected compiler manifest absent from pinned tree"

# 3. Even with a committed compiler-family manifest, a generated untracked
# launcher cannot fake a compiler execution receipt.
printf '<project></project>\n' > "$tmp/standin/pom.xml"
git -C "$tmp/standin" add pom.xml
git -C "$tmp/standin" -c user.email=test@example.invalid -c user.name=GateTest commit -qm manifest
cp "$tmp/fake-runner" "$tmp/standin/launcher"
chmod +x "$tmp/standin/launcher"
run_negative untracked-wrapper "$tmp/standin" "$tmp/standin/launcher" \
  "launcher is not tracked in pinned compiler tree"

echo "TRI_BACKEND_ADMISSION_NEGATIVE=PASS cases=3 (HOST_ONLY)"
