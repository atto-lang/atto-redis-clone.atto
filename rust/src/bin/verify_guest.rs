//! Fail-closed real guest execution gate: Rust never emulates Atto instructions.
//! Use a separately pinned, reviewed backend runner/compiler. No fake PASS.
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, message.into())
}

fn sha_of(path: &Path) -> io::Result<String> {
    if !path.is_file() || path.symlink_metadata()?.file_type().is_symlink() {
        return Err(invalid("guest source must be a regular, non-symlink file"));
    }
    let result = Command::new("git")
        .arg("hash-object").arg("--no-filters").arg(path).output()?;
    if !result.status.success() {
        return Err(invalid("git hash-object failed"));
    }
    String::from_utf8(result.stdout)
        .map(|s| s.trim().to_owned()).map_err(|_| invalid("invalid hash-object stdout"))
}

fn parse_args() -> io::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(),
            "--mode" | "--source" | "--source-blob" |
            "--expected-output" | "--expected-exit" | "--runner") {
            return Err(invalid(format!("unknown argument: {flag}")));
        }
        let value = args.next().ok_or_else(|| invalid(format!("missing value for {flag}")))?;
        if map.insert(flag, value).is_some() {
            return Err(invalid("duplicate argument"));
        }
    }
    Ok(map)
}

fn required<'a>(args: &'a HashMap<String, String>, flag: &str) -> io::Result<&'a str> {
    args.get(flag).map(String::as_str)
        .ok_or_else(|| invalid(format!("missing {flag}")))
}

fn check_output(status: ExitStatus, stdout: &[u8], expected: &[u8],
                expected_exit: i32) -> io::Result<()> {
    if stdout.len() > 1024 * 1024 {
        return Err(invalid("guest stdout exceeded one MiB admission budget"));
    }
    if status.code() != Some(expected_exit) || stdout != expected {
        return Err(invalid(format!(
            "real guest result mismatch: exit={:?}, expected={expected_exit}, stdout={:?}",
            status.code(), String::from_utf8_lossy(stdout)
        )));
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("NOT_CERTIFIED: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let args = parse_args()?;
    let mode = required(&args, "--mode")?;
    if mode != "java" && mode != "llvm" {
        return Err(invalid("mode must be java or llvm"));
    }
    let source = PathBuf::from(required(&args, "--source")?);
    let pin = required(&args, "--source-blob")?;
    if pin.len() != 40 || !pin.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid source Git blob SHA"));
    }
    let actual_sha = sha_of(&source)?;
    if actual_sha != pin {
        return Err(invalid(format!(
            "source blob drift: expected {pin}, actual {actual_sha}"
        )));
    }
    let expected = if let Some(path) = args.get("--expected-output") {
        fs::read(path)?
    } else {
        Vec::new()
    };
    if expected.len() > 1024 * 1024 {
        return Err(invalid("expected output too large"));
    }
    let expected_exit = args.get("--expected-exit")
        .map(|s| s.parse::<i32>())
        .transpose().map_err(|_| invalid("invalid expected-exit"))?
        .unwrap_or(0);
    let runner = match args.get("--runner") {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("NOT_EXECUTED: no reviewed, real Atto compiler/runner provided");
            std::process::exit(2);
        }
    };
    if !runner.is_absolute() || !runner.is_file() || runner.symlink_metadata()?.file_type().is_symlink() {
        return Err(invalid("runner must be an absolute, regular, non-symlink executable"));
    }
    let source = source.canonicalize()?;
    if mode == "java" {
        let result = Command::new(&runner).arg(&source).output()?;
        if !result.status.success() && result.status.code() != Some(expected_exit) {
            eprintln!("runner stderr: {}", String::from_utf8_lossy(&result.stderr));
        }
        check_output(result.status, &result.stdout, &expected, expected_exit)?;
    } else {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("system clock before epoch"))?.as_nanos();
        let binary = env::temp_dir().join(format!("atto-guest-{}-{stamp}", std::process::id()));
        let compile = Command::new(&runner).arg(&source).arg(&binary).output()?;
        if !compile.status.success() || !compile.stdout.is_empty() {
            let _ = fs::remove_file(&binary);
            return Err(invalid(format!(
                "LLVM real compiler rejected fixture: status={:?}, stderr={}",
                compile.status.code(), String::from_utf8_lossy(&compile.stderr)
            )));
        }
        if !binary.is_file() || binary.symlink_metadata()?.file_type().is_symlink() {
            return Err(invalid("LLVM compiler did not create a native binary"));
        }
        let execution = Command::new(&binary).output();
        let _ = fs::remove_file(&binary);
        let result = execution?;
        check_output(result.status, &result.stdout, &expected, expected_exit)?;
    }
    println!("GUEST_EXECUTED mode={mode} blob={actual_sha} exit={expected_exit} output_bytes={}", expected.len());
    Ok(())
}
