//! Source size, UTF-8 and argument admission in the real CLI.
#[path = "support/mod.rs"]
mod support;

fn case(bytes: &[u8], mode: &str, args: &[&str], diagnostic: &str) {
    let result = support::run_source(mode, bytes, args);
    assert_eq!(result.status.code(), Some(1), "invalid case was admitted or crashed");
    assert!(result.stdout.is_empty(), "rejection emitted success output");
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.starts_with("orescranelift: ") && error.contains(diagnostic),
        "wrong compiler-owned rejection: {error}"
    );
    assert!(
        !error.contains("panicked at") && !error.contains("stack backtrace"),
        "compiler panic is not a source-admission rejection"
    );
}

#[test]
fn invalid_utf8_is_rejected_before_frontend_or_jit() {
    case(b"fnc main():int{return 42;} // \xff", "run", &[],
        "source is not valid UTF-8");
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
        if invalid == "+1" { continue; }
        case(source, "run", &[invalid], "invalid i64 argument");
    }
}

#[test]
fn ir_mode_must_refuse_runtime_arguments() {
    case(b"fnc main(x:int):int{return x+1;}", "ir", &["1"],
        "ir mode does not accept arguments");
}

#[test]
fn runtime_arity_is_checked_before_native_execution() {
    case(b"fnc main(x:int):int{return x+1;}", "run", &[],
        "runtime argument count mismatch");
}
