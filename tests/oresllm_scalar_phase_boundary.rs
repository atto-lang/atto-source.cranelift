// Native Cranelift CLI phase separation. Compiler refusal must never
// masquerade as checked arithmetic overflow; a JIT fault must never become
// a successful parser-rejection case. Source-authored until cargo actually runs.
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_CASE: AtomicU64 = AtomicU64::new(0);

fn invoke(mode: &str, source: &str) -> std::process::Output {
    let nonce = NEXT_CASE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "oresllm-jit-phase-{}-{nonce}.ores",
        std::process::id()
    ));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    f.write_all(format!("pub fnc main(): int {{ return {source}; }}\n").as_bytes())
        .unwrap();
    drop(f);
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    result
}
fn compiler_rejects(expression: &str) {
    let ir = invoke("ir", expression);
    assert_eq!(
        ir.status.code(),
        Some(1),
        "{expression} must fail at compile time"
    );
    assert!(
        ir.stdout.is_empty(),
        "{expression} leaked generated native IR"
    );
    let err = String::from_utf8_lossy(&ir.stderr);
    assert!(
        err.starts_with("orescranelift: "),
        "{expression}: no compiler diagnostic: {err}"
    );
    assert!(
        !err.contains("i64 arithmetic overflow"),
        "{expression}: syntax rejection misreported as runtime overflow"
    );
    let guest = invoke("run", expression);
    assert_eq!(
        guest.status.code(),
        Some(1),
        "{expression}: runtime-mode acceptance"
    );
    assert!(
        guest.stdout.is_empty(),
        "{expression}: invalid source executed"
    );
}
fn runtime_overflows(expression: &str) {
    let ir = invoke("ir", expression);
    assert!(
        ir.status.success(),
        "{expression}: never reached native JIT lowering: {}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(!ir.stdout.is_empty(), "{expression}: empty Cranelift IR");
    assert!(
        ir.stderr.is_empty(),
        "{expression}: compiler warnings/errors leaked"
    );
    let guest = invoke("run", expression);
    assert_eq!(
        guest.status.code(),
        Some(1),
        "{expression}: overflow exited unexpectedly"
    );
    assert!(
        guest.stdout.is_empty(),
        "{expression}: overflow emitted guest stdout"
    );
    assert_eq!(
        String::from_utf8_lossy(&guest.stderr),
        "orescranelift: i64 arithmetic overflow\n",
        "{expression}: native overflow category differs"
    );
}
#[test]
fn shared_scalar_compile_rejections_are_not_jit_overflow() {
    for expression in ["unknown_value", "5 ^ 3", "5 ^^ 3", "1__0"] {
        compiler_rejects(expression);
    }
}
#[test]
fn shared_scalar_checked_i64_overflows_are_not_compiler_refusals() {
    for expression in [
        "9223372036854775807 + 1",
        "-9223372036854775808 - 1",
        "9223372036854775807 * 2",
        "-(-9223372036854775808)",
        "(9223372036854775807 + 1) * 0",
        "0 - (9223372036854775807 + 1)",
        "-9223372036854775808 / -1",
    ] {
        runtime_overflows(expression);
    }
}

#[test]
fn checked_quotient_has_guarded_ir_and_native_result() {
    let ir = invoke("ir", "7 / 2");
    assert!(ir.status.success());
    let clif = String::from_utf8_lossy(&ir.stdout);
    assert!(clif.contains("sdiv") && clif.matches("brif").count() >= 2);
    let guest = invoke("run", "7 / 2");
    assert!(guest.status.success());
    assert_eq!(String::from_utf8_lossy(&guest.stdout), "3\n");
    let zero = invoke("run", "7 / 0");
    assert_eq!(zero.status.code(), Some(1));
    assert!(zero.stdout.is_empty());
    assert!(String::from_utf8_lossy(&zero.stderr).starts_with("orescranelift: "));
}
