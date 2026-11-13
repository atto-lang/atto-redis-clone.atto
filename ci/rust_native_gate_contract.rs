//! Rust-only source integrity test for the fail-closed LLVM and .NET native
//! admission runner; it does NOT execute an Atto compiler or broker.
use std::fs;
fn runner() -> String {
    fs::read_to_string("ci/native-llvm-net-scalar.sh")
        .expect("read the real broker native-admission script")
}
#[test]
fn native_actor_gate_requires_rust_and_forbids_python() {
    let s = runner();
    assert!(s.contains("tests/source_actor_e2e.rs"));
    assert!(s.contains("rustc --edition=2021 -O -D warnings"));
    assert!(s.contains("LLVM_RUST_ACTOR_DRIVER_BLOB="));
    assert!(s.contains("LLVM Rust source actor suite missing"));
    assert!(!s.contains("python3"));
    assert!(!s.contains("source_actor_e2e.py"));
}
#[test]
fn native_actor_gate_keeps_all_positive_and_negative_receipts() {
    let s = runner();
    for evidence in [
        "LLVM_NATIVE_SOURCE_ACTOR_POSITIVE_CASES=",
        "LLVM_NATIVE_SOURCE_ACTOR_NEGATIVE_CASES=PASS",
        "LLVM_NATIVE_SOURCE_ACTORS_EXECUTED=PASS",
        "'^PASS native source actor:'",
        "'PASS current-source actor lowering and physical heap reclamation'",
        "'^PASS [1-9][0-9]* no-IR actor ownership and await boundary cases$'",
    ] {
        assert!(s.contains(evidence), "missing mandatory receipt: {evidence}");
    }
    assert!(s.contains("-ge 16"));
    assert!(s.contains("timeout --kill-after=10s 300s"));
}
#[test]
fn native_scalar_and_net_actor_negative_gates_cannot_be_replaced_by_broker_claims() {
    let s = runner();
    for guard in [
        "ATTO_APP_EXPECTED_SHA",
        "ATTO_APP_EXPECTED_TREE",
        "ATTO_LLVM_EXPECTED_SHA",
        "ATTO_LLVM_EXPECTED_TREE",
        "ATTO_NET_EXPECTED_SHA",
        "ATTO_NET_EXPECTED_TREE",
        "NET_NATIVE_ACTOR_",
        "REJECTED_AS_EXPECTED",
        "ATTO_NATIVE_ACTOR_HEAP_CERTIFIED=false",
        "ATTO_NATIVE_NONBLOCKING_BROKER_CERTIFIED=false",
        "ATTO_NATIVE_DURABILITY_CERTIFIED=false",
    ] {
        assert!(s.contains(guard), "missing fail-closed admission guard {guard}");
    }
}
