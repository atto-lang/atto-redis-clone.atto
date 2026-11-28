//! Arithmetic gate for independently measured, comparable native Redis broker runs.
//! The input files are UNTRUSTED assertions: this does not authenticate compiler
//! execution, code identity, runner identity, disk policy or a real Atto guest.
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::File;
use std::io::{self, Read};

const MIN_SAMPLES: usize = 5;
const MAX_SAMPLES: usize = 20;
const MAX_RECEIPT_BYTES: u64 = 65_536;
const MIN_RATIO: f64 = 0.70;
const REQUIRED: [&str; 17] = [
    "format",
    "broker",
    "backend",
    "workload",
    "scope",
    "durability",
    "ack_policy",
    "clients",
    "payload_bytes",
    "machine_id",
    "source_commit",
    "compiler_commit",
    "executed_steps",
    "correctness",
    "native_guest",
    "nonblocking_io",
    "samples_ops_sec",
];

#[derive(Debug)]
struct Receipt {
    fields: BTreeMap<String, String>,
    samples: Vec<f64>,
}

fn invalid(reason: &str) -> String {
    format!("PERF_GATE_REJECT: {reason}")
}

fn positive_u64(text: &str, field: &str) -> Result<u64, String> {
    let n: u64 = text.parse().map_err(|_| invalid(field))?;
    if n == 0 {
        return Err(invalid(field));
    }
    Ok(n)
}

fn sha40(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse(text: &str) -> Result<Receipt, String> {
    if text.len() > MAX_RECEIPT_BYTES as usize {
        return Err(invalid("receipt exceeds byte cap"));
    }
    let mut fields = BTreeMap::new();
    for raw in text.lines() {
        if raw.is_empty() {
            continue;
        }
        let (key, value) = raw
            .split_once('=')
            .ok_or_else(|| invalid("missing key=value separator"))?;
        if !REQUIRED.contains(&key) || value.is_empty() || value.trim() != value {
            return Err(invalid("unknown key or malformed value"));
        }
        if fields.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(invalid("duplicate receipt key"));
        }
    }
    let present: BTreeSet<&str> = fields.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = REQUIRED.into_iter().collect();
    if present != expected {
        return Err(invalid("missing receipt fields"));
    }
    let get = |key: &str| fields.get(key).map(String::as_str).unwrap_or("");
    if get("format") != "atto-redis-native-perf-v1" || get("broker") != "redis" {
        return Err(invalid("incorrect contract or broker"));
    }
    if get("scope") != "full_resp2_tcp_broker" {
        return Err(invalid("scalar/preface/host-model benchmark is not a broker"));
    }
    if !["fsync_always", "fsync_everysec"].contains(&get("durability")) {
        return Err(invalid("durable storage mode required"));
    }
    if !sha40(get("source_commit")) || !sha40(get("compiler_commit")) {
        return Err(invalid("invalid source/compiler commit shape"));
    }
    if get("machine_id").len() > 128 || get("machine_id").is_empty() {
        return Err(invalid("machine ID"));
    }
    for field in ["clients", "payload_bytes", "executed_steps"] {
        positive_u64(get(field), field)?;
    }
    if get("correctness") != "PASS" {
        return Err(invalid("correctness must pass before timing"));
    }
    if !["true", "false"].contains(&get("native_guest"))
        || !["true", "false"].contains(&get("nonblocking_io"))
    {
        return Err(invalid("invalid guest/io status"));
    }
    let samples: Vec<f64> = get("samples_ops_sec")
        .split(',')
        .map(|value| {
            let n: f64 = value.parse().map_err(|_| invalid("invalid sample"))?;
            if !n.is_finite() || n <= 0.0 {
                return Err(invalid("non-positive or non-finite sample"));
            }
            Ok(n)
        })
        .collect::<Result<_, _>>()?;
    if !(MIN_SAMPLES..=MAX_SAMPLES).contains(&samples.len()) {
        return Err(invalid("at least five and at most twenty trials required"));
    }
    Ok(Receipt { fields, samples })
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let m = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        sorted[m - 1] / 2.0 + sorted[m] / 2.0
    } else {
        sorted[m]
    }
}

fn field<'a>(receipt: &'a Receipt, key: &str) -> &'a str {
    receipt.fields.get(key).map(String::as_str).unwrap_or("")
}

fn evaluate(atto: &Receipt, reference: &Receipt) -> Result<f64, String> {
    if !["java", "llvm", "dotnet"].contains(&field(atto, "backend"))
        || field(reference, "backend") != "rust"
    {
        return Err(invalid("expected real Atto backend versus Rust reference"));
    }
    if field(atto, "native_guest") != "true" || field(atto, "nonblocking_io") != "true" {
        return Err(invalid("Atto native guest and nonblocking IO required"));
    }
    if field(reference, "native_guest") != "false" {
        return Err(invalid("reference cannot be substituted for Atto guest"));
    }
    for key in [
        "format",
        "broker",
        "workload",
        "scope",
        "durability",
        "ack_policy",
        "clients",
        "payload_bytes",
        "machine_id",
    ] {
        if field(atto, key) != field(reference, key) {
            return Err(invalid("benchmark configurations do not match"));
        }
    }
    let ratio = median(&atto.samples) / median(&reference.samples);
    if !ratio.is_finite() {
        return Err(invalid("invalid median ratio"));
    }
    if ratio < MIN_RATIO {
        return Err(format!(
            "PERF_GATE_BELOW_TARGET ratio={ratio:.6} minimum={MIN_RATIO:.2}"
        ));
    }
    Ok(ratio)
}

fn read_receipt(path: &str) -> Result<Receipt, String> {
    let file = File::open(path).map_err(|e| invalid(&format!("open receipt: {e}")))?;
    let mut limited = file.take(MAX_RECEIPT_BYTES + 1);
    let mut text = String::new();
    limited
        .read_to_string(&mut text)
        .map_err(|e| invalid(&format!("read receipt: {e}")))?;
    parse(&text)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 || args[1] != "--atto" || args[3] != "--reference" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: parity_gate --atto ATTO_RECEIPT --reference RUST_RECEIPT",
        )
        .into());
    }
    let atto = read_receipt(&args[2])?;
    let reference = read_receipt(&args[4])?;
    let ratio = evaluate(&atto, &reference)?;
    println!("PERF_ARITHMETIC_PASS_NOT_CERTIFIED ratio={ratio:.6} minimum={MIN_RATIO:.2}");
    println!("PROVENANCE_UNVERIFIED=true REAL_GUEST_UNVERIFIED=true");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(backend: &str, samples: &str) -> String {
        format!(
            "format=atto-redis-native-perf-v1\n\
             broker=redis\n\
             backend={backend}\n\
             workload=resp2_set50_get40_del10\n\
             scope=full_resp2_tcp_broker\n\
             durability=fsync_always\n\
             ack_policy=after_fsync\n\
             clients=8\n\
             payload_bytes=64\n\
             machine_id=isolated-runner-1\n\
             source_commit={}\n\
             compiler_commit={}\n\
             executed_steps=12\n\
             correctness=PASS\n\
             native_guest={}\n\
             nonblocking_io=true\n\
             samples_ops_sec={samples}\n",
            "a".repeat(40),
            "b".repeat(40),
            if backend == "rust" { "false" } else { "true" },
        )
    }

    #[test]
    fn exactly_seventy_percent_passes_and_below_fails() {
        let reference = parse(&fixture("rust", "100,100,100,100,100")).unwrap();
        let atto = parse(&fixture("llvm", "70,70,70,70,70")).unwrap();
        assert_eq!(evaluate(&atto, &reference).unwrap(), 0.7);
        let slow = parse(&fixture("dotnet", "69,69,69,69,69")).unwrap();
        assert!(evaluate(&slow, &reference).is_err());
    }

    #[test]
    fn each_mismatch_fails_closed() {
        let reference = parse(&fixture("rust", "100,100,100,100,100")).unwrap();
        let original = fixture("java", "90,90,90,90,90");
        for replacement in [
            "durability=fsync_everysec",
            "scope=preface",
            "machine_id=other-runner",
            "clients=16",
            "ack_policy=before_fsync",
        ] {
            let changed = original.replace(
                if replacement.starts_with("durability") {
                    "durability=fsync_always"
                } else if replacement.starts_with("scope") {
                    "scope=full_resp2_tcp_broker"
                } else if replacement.starts_with("machine_id") {
                    "machine_id=isolated-runner-1"
                } else if replacement.starts_with("clients") {
                    "clients=8"
                } else {
                    "ack_policy=after_fsync"
                },
                replacement,
            );
            assert!(parse(&changed).and_then(|r| evaluate(&r, &reference)).is_err());
        }
    }

    #[test]
    fn missing_duplicate_invalid_and_synthetic_receipts_reject() {
        let base = fixture("java", "90,90,90,90,90");
        assert!(parse(&base.replace("executed_steps=12\n", "")).is_err());
        assert!(parse(&(base.clone() + "clients=8\n")).is_err());
        assert!(parse(&base.replace("executed_steps=12", "executed_steps=0")).is_err());
        assert!(parse(&base.replace("correctness=PASS", "correctness=HOST_ONLY")).is_err());
        assert!(parse(&base.replace("samples_ops_sec=90,90,90,90,90", "samples_ops_sec=NaN,90,90,90,90")).is_err());
        assert!(parse(&base.replace("samples_ops_sec=90,90,90,90,90", "samples_ops_sec=90")).is_err());
        let reference = parse(&fixture("rust", "100,100,100,100,100")).unwrap();
        let fake = parse(&base.replace("native_guest=true", "native_guest=false")).unwrap();
        assert!(evaluate(&fake, &reference).is_err());
    }

    #[test]
    fn median_rejects_outlier_bias_without_using_fastest_trial() {
        let reference = parse(&fixture("rust", "100,100,100,100,100")).unwrap();
        let atto = parse(&fixture("llvm", "1,69,69,69,99999")).unwrap();
        assert!(evaluate(&atto, &reference).is_err());
        assert_eq!(median(&[1.0, 4.0, 8.0, 10.0]), 6.0);
    }
}
