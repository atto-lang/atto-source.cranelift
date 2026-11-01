use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

// These are REAL Cranelift JIT execution assertions, not an arithmetic host oracle.
// Their exact expression/expected-value inventory mirrors scalar.json and must
// be reconciled with the Java/LLVM/WASM/.NET source-execution results.
fn run_case(expression: &str, expected: Option<&str>) {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "oresllm-cranelift-parity-{}-{sequence}.ores",
        std::process::id()
    ));
    // Exclusive file creation prevents symlink-clobbering or fixture collisions.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(format!("pub fnc main(): int {{ return {expression}; }}\n").as_bytes())
        .unwrap();
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    match expected {
        Some(value) => {
            assert!(
                output.status.success(),
                "{expression}: expected native success; stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                format!("{value}\n"),
                "{expression}: native result mismatch"
            );
        }
        None => {
            assert_eq!(output.status.code(), Some(1),
                "{expression}: expected controlled CLI rejection (exit 1), got {:?}",
                output.status);
            assert!(output.stdout.is_empty(), "{expression}: rejection leaked stdout");
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(diagnostic.starts_with("orescranelift: "),
                "{expression}: non-compiler failure or crash was miscounted: {diagnostic}");
        }
    }
}

#[test]
fn oresllm_shared_scalar_positive_native_jit() {
    for (expression, expected) in [
        ("0", "0"),
        ("42", "42"),
        ("2 + 3 * 4", "14"),
        ("(2 + 3) * 4", "20"),
        ("-7 + 3", "-4"),
        ("-7 * -6", "42"),
        ("9223372036854775807", "9223372036854775807"),
        ("-9223372036854775808", "-9223372036854775808"),
        ("9223372036854775807 - 1", "9223372036854775806"),
        ("-9223372036854775808 + 1", "-9223372036854775807"),
        ("1 - 9", "-8"),
        ("(-5) * (3 + 4)", "-35"),
        ("(17 - 2) * 3", "45"),
        ("0 - (4 * 5)", "-20"),
        ("1 + (2 * (3 + 4))", "15"),
        ("-(-5)", "5"),
        ("0 * 9223372036854775807", "0"),
    ] {
        run_case(expression, Some(expected));
    }
}

#[test]
fn oresllm_checked_overflow_must_fail_without_stdout() {
    for expression in [
        "9223372036854775807 + 1",
        "-9223372036854775808 - 1",
        "9223372036854775807 * 2",
        "-(-9223372036854775808)",
        "(9223372036854775807 + 1) * 0",
        "0 - (9223372036854775807 + 1)",
    ] {
        run_case(expression, None);
    }
}

#[test]
fn oresllm_invalid_scalar_source_must_not_execute() {
    for expression in [
        "unknown_value",
        "7 / 2",
        "5 ^ 3",
        "5 ^^ 3",
        "1__0",
    ] {
        run_case(expression, None);
    }
}

#[test]
fn supplemental_scalar_arithmetic_executes_through_real_jit() {
    // Extra native execution checks; intentionally not counted as shared-manifest vectors.
    for (expression, expected) in [
        ("-7 * 0", "0"),
        ("(2 + 3) * (10 - 2)", "40"),
        ("9223372036854775807 - 9223372036854775807", "0"),
    ] {
        run_case(expression, Some(expected));
    }
}
