#!/usr/bin/env python3
"""Static regression gate for the draft tagged Atto Redis guest.

This checks lexical sealed-state discipline; it is NOT a compiler, execution,
concurrency, or cross-backend certification.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
GUEST = ROOT / "guest" / "redis_actor_nio_multiclient_tagged.atto"
LOAN = re.compile(
    r"self\.handle_state\(critical-(read|write)\s+\|s\|\s*->\s*\{(.*?)\}\s*\);",
    re.DOTALL,
)


def check(text: str) -> None:
    source = re.sub(r"//[^\n]*", "", text)
    state_blocks = re.findall(r"\bstate\s*\{\s*let\s+Map<Bytes,Bytes>\s+values\s*;\s*\}", source)
    assert len(state_blocks) == 1, "expected one sealed mutable Bytes map schema"
    assert not re.search(r"\bself\.values\b", source), "direct actor state access must fail"
    loans = list(LOAN.finditer(source))
    assert [m.group(1) for m in loans] == ["write", "write", "read", "write"], (
        "expected critical-write for initialization/SET/DEL and critical-read for GET"
    )
    write_ops = ("s.values = ~map<Bytes,Bytes>()", "s.values.set(key, value)", "s.values.delete(key)")
    read_ops = ("s.values.get_optional(key)",)
    for m in loans:
        mode, body = m.group(1), m.group(2)
        assert "self.send(" not in body, "do not publish while holding a state loan"
        assert "await " not in body and "rt cooperate" not in body, "a state loan must not suspend"
        if mode == "read":
            assert all(op not in body for op in write_ops), "critical-read cannot mutate state"
        else:
            assert all(op not in body for op in read_ops), "GET must use critical-read"
    outside = LOAN.sub("", source)
    assert "s.values" not in outside, "a borrowed state view escaped its lexical scope"
    assert all(any(op in m.group(2) and m.group(1) == "write" for m in loans) for op in write_ops), (
        "every write operation must be guarded by critical-write"
    )
    assert all(any(op in m.group(2) and m.group(1) == "read" for m in loans) for op in read_ops), (
        "map lookup must be guarded by critical-read"
    )


def main() -> None:
    source = GUEST.read_text(encoding="utf-8")
    check(source)
    # Mutations ensure the static guard itself fails closed on common regressions.
    regressions = {
        "unsealed field": source.replace("state {\n    let Map<Bytes,Bytes> values;\n  }", "let Map<Bytes,Bytes> values;", 1),
        "direct access": source.replace("s.values.set(key, value)", "self.values.set(key, value)", 1),
        "write lookup": source.replace("critical-read |s|", "critical-write |s|", 1),
        "read mutation": source.replace("critical-write |s| -> {\n        s.values.set", "critical-read |s| -> {\n        s.values.set", 1),
        "unscoped delete": source.replace("s.values.delete(key)", "NOT_SCOPED_DELETE(key)", 1),
        "publish under loan": source.replace("s.values.set(key, value);", "s.values.set(key, value);\n        self.send((client_id, request_id, \"OK\", value));", 1),
    }
    for name, broken in regressions.items():
        assert broken != source, f"fixture for {name} did not mutate source"
        try:
            check(broken)
        except AssertionError:
            continue
        raise AssertionError(f"static checker failed to reject {name}")
    print("PASS: tagged Atto Redis sealed-state lexical source contract + 6 negative probes")
    

if __name__ == "__main__":
    main()
