//! Real stage-0 CLI admission; still no compiled singleton guest.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn run(source: &str, mode: &str, args: &[&str]) -> std::process::Output {
    let n = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-singleton-admission-{}-{n}.atto",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    out
}

#[test]
fn singleton_declaration_rejected_in_shipping_run_and_ir() {
    for declaration in [
        "define singleton S as end",
        "define pub singleton S as end",
        "pub define singleton S as end",
        "define // inert\r singleton S as end",
        "define singleton S implements Stateful<S> as end",
    ] {
        for mode in ["run", "ir"] {
            let result = run(declaration, mode, &[]);
            assert!(!result.status.success(), "{mode} admitted {declaration}");
            assert!(
                result.stdout.is_empty(),
                "{mode} emitted code for {declaration}"
            );
            let err = String::from_utf8_lossy(&result.stderr);
            assert!(err.contains("ORES-SINGLETON-UNSUPPORTED"), "{err}");
            assert!(err.contains("DECL_KIND_UNSUPPORTED"), "{err}");
        }
    }
}

#[test]
fn ordinary_singleton_parameter_is_not_declaration_syntax() {
    let program = "fnc main(singleton:int):int{return singleton;}";
    let run_output = run(program, "run", &["7"]);
    assert!(
        run_output.status.success(),
        "{}",
        String::from_utf8_lossy(&run_output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run_output.stdout).trim(), "7");
    let ir_output = run(program, "ir", &[]);
    assert!(
        ir_output.status.success(),
        "{}",
        String::from_utf8_lossy(&ir_output.stderr)
    );
    assert!(!ir_output.stdout.is_empty());
}
