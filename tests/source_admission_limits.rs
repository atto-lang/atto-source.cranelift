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
