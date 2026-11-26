//! Additional REAL Cranelift JIT differential regressions for the
//! OresLLM checked-i64 intersection. No Java/LLVM/WASM/.NET parity is inferred.
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn cli(source: &str, mode: &str) -> std::process::Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "oresllm-jit-adversarial-{}-{id}.ores",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .expect("exclusive fixture");
    file.write_all(source.as_bytes()).expect("write fixture");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("launch actual Cranelift CLI");
    std::fs::remove_file(path).expect("remove fixture");
    output
}

fn guest(expression: &str) -> String {
    format!("pub fnc main(): int {{ return {expression}; }}\n")
}
fn accept(expression: &str, expected: i64) {
    let output = cli(&guest(expression), "run");
    assert!(output.status.success(), "{expression}: {:?}", output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{expected}\n"),
        "different native result for {expression}"
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected native diagnostic for {expression}"
    );
}
fn refuse(expression: &str) {
    for mode in ["run", "ir"] {
        let output = cli(&guest(expression), mode);
        assert_eq!(
            output.status.code(),
            Some(1),
            "unsupported or overflow expression admitted by {mode}: {expression}"
        );
        assert!(output.stdout.is_empty(), "rejected {mode} leaked output");
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "),
            "rejected {mode} must produce controlled compiler diagnostic: {expression}"
        );
    }
}
fn draw(state: &mut u64) -> i64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*state >> 16) % 2001) as i64 - 1000
}

#[test]
fn seeded_native_checked_i64_differential_96_cases() {
    // Unlike a host BigInt model, these 96 cases actually launch compiled JIT code.
    let mut state: u64 = 0x8b8b_1bee_5ad1_2026;
    for _ in 0..96 {
        let a = draw(&mut state);
        let b = draw(&mut state);
        let c = draw(&mut state);
        accept(&format!("({a} + {b}) * {c}"), (a + b) * c);
        accept(&format!("{a} - ({b} * {c})"), a - b * c);
    }
}

#[test]
fn eager_checked_overflow_cannot_hide_under_zero_or_bitxor() {
    // Emitting a multiplication by zero must NOT skip an overflowing operand.
    for expr in [
        "0 * (9223372036854775807 + 1)",
        "(9223372036854775807 + 1) * 0",
        "Integer.bitXor(1, 9223372036854775807 + 1)",
        "Integer.bitXor(9223372036854775807 + 1, 0)",
        "0 - (-9223372036854775808 - 1)",
        "(-9223372036854775808) * -1",
    ] {
        // Compile-only IR can legitimately be emitted for runtime overflow,
        // so reject only actual execution for this category.
        let output = cli(&guest(expr), "run");
        assert_eq!(output.status.code(), Some(1), "{expr} incorrectly ran");
        assert!(output.stdout.is_empty(), "{expr} leaked a success result");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "));
    }
}

#[test]
fn unsupported_ownership_actor_and_decimal_source_never_emits_ir() {
    for expression in [
        "rt mut share 1",
        "rt borrow x",
        "new Actor()",
        "1.5",
        "1.0 / 2.0",
        "1__0",
        "1 ^ 2",
        "1 ^^ 2",
        "unknown_scalar",
    ] {
        refuse(expression);
    }
}

#[test]
fn checked_quotient_failures_do_not_publish_execution_results() {
    // Compilation of guarded division is permitted; execution must reject.
    for expr in ["1 / 0", "-9223372036854775808 / -1"] {
        let output = cli(&guest(expr), "run");
        assert_eq!(output.status.code(), Some(1), "{expr} incorrectly ran");
        assert!(output.stdout.is_empty(), "{expr} leaked a success result");
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "));
    }
}
