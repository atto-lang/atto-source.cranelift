//! Compiler CLI admission checks. Real Cranelift binary, not a regex oracle.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn cli(source: &str, mode: &str) -> std::process::Output {
    let id = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path=std::env::temp_dir().join(format!(
        "ores-cranelift-critical-{}-{id}.ores",std::process::id()));
    std::fs::write(&path,source).unwrap();
    let output=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode).arg(&path).output().unwrap();
    std::fs::remove_file(&path).unwrap();
    output
}
#[test]
fn modifier_rejections_have_specific_diagnostics_and_no_ir_or_guest_output() {
    for (source,diagnostic) in [
        ("async fnc main():int{return 42;}","E_CALLABLE_MODIFIERS"),
        ("critical suspends fnc main():int{return 42;}","E_CALLABLE_MODIFIERS"),
        ("suspends critical fnc main():int{return 42;}","E_CALLABLE_MODIFIERS"),
        ("pub critical fnc main():int{return 42;}","E_CRITICAL_UNSUPPORTED"),
        ("define pub suspends fnc main():int{return 42;}","E_SUSPENDS_UNSUPPORTED"),
        ("fnc main():int{critical {return 42;}}","E_CRITICAL_UNSUPPORTED"),
        ("fnc main():int{try critical {return 42;} else {return 0;}}","E_CRITICAL_UNSUPPORTED"),
        ("fnc main():int{return await 42;}","E_CRITICAL_UNSUPPORTED"),
    ] {
        for mode in ["run","ir"] {
            let result=cli(source,mode);
            assert_eq!(result.status.code(),Some(1),"{mode}: {source}");
            assert!(result.stdout.is_empty(),"{mode}: leaked generated IR or guest output");
            let stderr=String::from_utf8_lossy(&result.stderr);
            assert!(stderr.contains(diagnostic),"{mode}: {source}: {stderr}");
            assert!(!stderr.contains("panicked"),"{mode}: compiler panic: {stderr}");
        }
    }
}
#[test]
fn comments_cannot_inject_rejected_modifiers_and_scalar_guest_still_executes() {
    let source="fnc main():int{return 40+2;} // critical suspends async await\n";
    let output=cli(source,"run");
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout,b"42\n");
}
