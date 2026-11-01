// Execute actual stage-0 Cranelift JIT machine code, never substitute host BigInt.
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_case(expression: &str, expected: Option<&str>) {
    let index = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "oresllm-budget-parity-{}-{index}.ores",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("exclusive source fixture");
    file.write_all(format!("pub fnc main(): int {{ return {expression}; }}\n").as_bytes())
        .expect("source fixture write");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("native JIT process");
    std::fs::remove_file(&path).expect("remove temporary source");
    if let Some(expected) = expected {
        assert!(
            output.status.success(),
            "{expression}: JIT guest failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{expected}\n"),
            "JIT guest produced incorrect signed-i64 output"
        );
    } else {
        assert!(!output.status.success(), "{expression}: invalid guest admitted");
        assert!(output.stdout.is_empty(), "{expression}: rejection leaked stdout");
        assert!(!output.stderr.is_empty(), "{expression}: missing diagnostic");
    }
}

#[test]
fn budget_additional_positive_guest_jit() {
    for (expression, expected) in [
        ("100 - (32 + 16)", "52"),
        ("9223372036854775807 - (2 * 64)", "9223372036854775679"),
        ("17 - 8 - 3", "6"),
        ("(-3) * (-7)", "21"),
        ("-9223372036854775808 + 0", "-9223372036854775808"),
    ] {
        run_case(expression, Some(expected));
    }
}

#[test]
fn budget_additional_checked_overflow_is_not_a_wraparound() {
    for expression in [
        "9223372036854775807 - (-1)",
        "-9223372036854775808 * -1",
        "0 * (9223372036854775807 + 1)",
    ] {
        run_case(expression, None);
    }
}

#[test]
fn budget_additional_invalid_syntax_never_executes() {
    for expression in ["1 + )", "(1 + 2", "1 2"] {
        run_case(expression, None);
    }
}
