use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(0);

fn compile_source(mode: &str, source: &str, args: &[&str]) -> std::process::Output {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-cranelift-singleton-{}-{n}.atto", std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode).arg(&path).args(args).output().unwrap();
    std::fs::remove_file(&path).unwrap();
    output
}

#[test]
fn singleton_declarations_fail_closed_in_run_and_ir_before_codegen() {
    for source in [
        "define singleton Settings as const int x = 1; end",
        "define pub singleton Settings as const int x = 1; end",
        "pub define singleton Settings as const int x = 1; end",
    ] {
        for mode in ["run", "ir"] {
            let output = compile_source(mode, source, &[]);
            assert!(!output.status.success(), "incorrectly accepted {mode}: {source}");
            assert!(output.stdout.is_empty(), "guest emitted output for unsupported singleton");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains("SINGLETON_UNSUPPORTED"), "{source}: {error}");
        }
    }
}

#[test]
fn comments_and_identifiers_are_not_false_singleton_declarations() {
    for source in [
        "// define singleton Cache as\nfnc main(singleton:int):int{return singleton+1;}",
        "/* define singleton Cache as */ fnc main(singleton:int):int{return singleton+1;}",
        "fnc singleton(x:int):int{return x+1;}",
    ] {
        let output = compile_source("run", source, &["41"]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
    }
}
