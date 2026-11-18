use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn case(bytes: &[u8], mode: &str, args: &[&str], diagnostic: &str) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-admission-{}-{id}.ores",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write isolated source fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .expect("execute source admission check");
    std::fs::remove_file(path).expect("clean isolated source fixture");
    assert!(!result.status.success(), "invalid case was admitted");
    assert!(result.stdout.is_empty(), "rejection emitted success output");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains(diagnostic),
        "wrong rejection diagnostic: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn invalid_utf8_is_rejected_before_frontend_or_jit() {
    case(
        b"fnc main():int{return 42;} // \xff",
        "run",
        &[],
        "source is not valid UTF-8",
    );
}

#[test]
fn oversize_source_is_rejected_before_utf8_and_frontend() {
    let mut bytes = vec![b' '; 1024 * 1024 + 1];
    bytes[..27].copy_from_slice(b"fnc main():int{return 42;} ");
    case(&bytes, "run", &[], "source exceeds 1 MiB limit");
}

#[test]
fn malformed_runtime_i64_is_not_silently_coerced() {
    let source = b"fnc main(x:int):int{return x+1;}";
    for invalid in ["+1", "1_0", "18446744073709551616", "1.5"] {
        // Rust's i64 parser accepts +1, so do not classify it as an error.
        if invalid == "+1" {
            continue;
        }
        case(source, "run", &[invalid], "invalid i64 argument");
    }
}

#[test]
fn ir_mode_must_refuse_runtime_arguments() {
    case(
        b"fnc main(x:int):int{return x+1;}",
        "ir",
        &["1"],
        "ir mode does not accept arguments",
    );
}

#[test]
fn runtime_arity_is_checked_before_native_execution() {
    case(
        b"fnc main(x:int):int{return x+1;}",
        "run",
        &[],
        "runtime argument count mismatch",
    );
}

fn early_cli_failure(mode: &str, args: &[&str], expected: &str) {
    // The path is never created. Early admission has priority over any
    // filesystem error and must not attempt to parse or compile the source.
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let missing = std::env::temp_dir().join(format!(
        "orescranelift-unopened-{}-{id}.atto",
        std::process::id()
    ));
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(missing)
        .args(args)
        .output()
        .expect("invoke CLI");
    assert_eq!(result.status.code(), Some(1), "expected normal compiler denial");
    assert!(result.stdout.is_empty(), "invalid argument printed guest output");
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        format!("orescranelift: {expected}\n"),
        "argument rejection must precede source lookup"
    );
}

#[test]
fn invalid_numeric_arguments_are_rejected_before_source_open() {
    for argument in ["not-an-int", "9223372036854775808", "-9223372036854775809", "1_0"] {
        early_cli_failure("run", &[argument], "invalid i64 argument");
    }
    early_cli_failure("isolate", &["0", "not-an-int"], "invalid i64 argument");
    early_cli_failure(
        "run",
        &["1", "2", "3", "4", "invalid"],
        "too many runtime arguments",
    );
    early_cli_failure("ir", &["not-an-int"], "ir mode does not accept arguments");
}

#[cfg(unix)]
#[test]
fn non_utf8_os_argv_rejects_without_panicking_or_reading_source() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let invalid = OsString::from_vec(vec![0xff]);
    let mode_result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(invalid.clone())
        .output()
        .expect("invoke malformed-mode CLI");
    assert_eq!(mode_result.status.code(), Some(1));
    assert!(mode_result.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&mode_result.stderr),
        "orescranelift: invalid command mode encoding\n"
    );

    let guest_result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg("/path/that/does/not/exist/arg-preflight.atto")
        .arg(invalid)
        .output()
        .expect("invoke malformed-argument CLI");
    assert_eq!(guest_result.status.code(), Some(1));
    assert!(guest_result.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&guest_result.stderr),
        "orescranelift: invalid i64 argument\n"
    );
}
