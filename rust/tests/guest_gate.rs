// This is a source-admission test of the Rust gate only.
// It explicitly does NOT run the Atto guest or certify a compiler.
use std::path::Path;
use std::process::Command;

fn fixture() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../guest/kv_cell.atto")
        .to_str().expect("UTF-8 fixture path").to_owned()
}

#[test]
fn missing_compiler_is_not_a_pass() {
    let child = Command::new(env!("CARGO_BIN_EXE_verify_guest"))
        .args([
            "--mode", "java",
            "--source", &fixture(),
            "--source-blob", "6a075fc0f0f36a4bb04cebc98dabfb183898f43f",
        ])
        .output().expect("run gate");
    assert_eq!(child.status.code(), Some(2), "no native runner must never be PASS");
    assert!(String::from_utf8_lossy(&child.stderr).contains("NOT_EXECUTED"));
}

#[test]
fn modified_fixture_fails_before_any_execution() {
    let child = Command::new(env!("CARGO_BIN_EXE_verify_guest"))
        .args([
            "--mode", "java",
            "--source", &fixture(),
            "--source-blob", "0000000000000000000000000000000000000000",
        ])
        .output().expect("run gate");
    assert_eq!(child.status.code(), Some(1), "source drift must fail closed");
    assert!(String::from_utf8_lossy(&child.stderr).contains("source blob drift"));
}

#[test]
fn invalid_backend_fails_without_guest_execution() {
    let child = Command::new(env!("CARGO_BIN_EXE_verify_guest"))
        .args([
            "--mode", "unverified",
            "--source", &fixture(),
            "--source-blob", "6a075fc0f0f36a4bb04cebc98dabfb183898f43f",
        ])
        .output().expect("run gate");
    assert_eq!(child.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&child.stderr).contains("mode must be java or llvm"));
}
