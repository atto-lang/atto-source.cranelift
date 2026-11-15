//! Real Cranelift scalar CLI; unsupported ownership is denied before native JIT.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_source(source: &str, mode: &str) -> Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-ownership-bang-{}-{sequence}.atto",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    output
}

#[test]
fn rejects_ownership_prefixes_before_run_or_ir() {
    for expression in [
        "copy!42", "take!42", "borrow! value", "borrow!mut value",
        "borrow! mut value", "share!value",
    ] {
        for mode in ["run", "ir"] {
            let source = format!("fnc main():int{{return {expression};}}");
            let output = run_source(&source, mode);
            assert!(!output.status.success(), "{mode} accepted {expression}");
            assert!(output.stdout.is_empty(), "{mode} emitted output for {expression}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("OWNERSHIP_BANG_BACKEND_UNSUPPORTED"),
                "{mode} wrong diagnostic for {expression}: {stderr}");
        }
    }
}

#[test]
fn commented_ownership_does_not_poison_scalar_execution() {
    for mode in ["run", "ir"] {
        let output = run_source(
            "fnc main():int{/* borrow! mut item */return 42;}",
            mode,
        );
        assert!(output.status.success(), "{mode}: {}",
            String::from_utf8_lossy(&output.stderr));
        assert!(!output.stdout.is_empty(), "{mode} produced no output");
    }
}
