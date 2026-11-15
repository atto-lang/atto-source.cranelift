//! Native CLI regression: layout infrastructure is NOT guest class support.
//! These must reject before any CLIF/JIT emission until typed ownership exists.
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_source(source: &str, mode: &str, arguments: &[&str]) -> std::process::Output {
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-layout-{}-{seq}.atto",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create unique source fixture");
    file.write_all(source.as_bytes()).expect("write source");
    drop(file);
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(arguments)
        .output();
    std::fs::remove_file(&path).expect("remove source fixture");
    result.expect("run real native compiler")
}

#[test]
fn unsupported_guest_aggregate_and_dynamic_source_fails_before_ir() {
    let unsupported = [
        "define class Point as\n  int x\n  int y\nend\nfnc main():int{return 42;}",
        "define inverted class Point as\n  int x\nend\nfnc main():int{return 42;}",
        "define actor Counter as\n  int state\nend\nfnc main():int{return 42;}",
        "fnc main():int{return Array.of(1,2,3);}",
        "fnc main():int{return Type.of(42);}",
        "fnc main():int{return eval(42);}",
    ];
    for source in unsupported {
        for mode in ["run", "ir"] {
            let result = run_source(source, mode, &[]);
            assert_eq!(
                result.status.code(),
                Some(1),
                "{mode} must reject a not-yet-implemented guest feature: {source}"
            );
            assert!(
                result.stdout.is_empty(),
                "{mode} must not print guest results or CLIF before rejection"
            );
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.starts_with("orescranelift: "),
                "must be a compiler-owned failure, got: {diagnostic}"
            );
            assert!(!diagnostic.contains("panicked at"));
        }
    }
}

#[test]
fn scalar_native_execution_remains_unaffected() {
    let result = run_source("fnc main(x:int):int{return x+2;}", "run", &["40"]);
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(result.stdout, b"42\n");
    assert!(result.stderr.is_empty());
}
