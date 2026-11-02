use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn native_execution(extension: &str, source: &str) -> std::process::Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "attolang-cranelift-{}-{id}.{extension}",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("create source fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("run the actual Cranelift guest compiler");
    std::fs::remove_file(path).expect("clean up source fixture");
    result
}

#[test]
fn atto_and_legacy_ores_scalar_sources_execute_with_identical_results() {
    const SOURCE: &str = "fnc main():int{return (6+1)*6;}";
    let atto = native_execution("atto", SOURCE);
    let ores = native_execution("ores", SOURCE);
    assert!(
        atto.status.success(),
        "{}",
        String::from_utf8_lossy(&atto.stderr)
    );
    assert!(
        ores.status.success(),
        "{}",
        String::from_utf8_lossy(&ores.stderr)
    );
    assert_eq!(atto.stdout, b"42\n");
    assert_eq!(atto.stdout, ores.stdout);
    assert_eq!(atto.status.code(), ores.status.code());
}

#[test]
fn atto_and_ores_both_fail_closed_for_unsupported_guest_syntax() {
    const SOURCE: &str = "define actor Unsupported as end";
    let atto = native_execution("atto", SOURCE);
    let ores = native_execution("ores", SOURCE);
    for output in [&atto, &ores] {
        assert!(
            !output.status.success(),
            "Unsupported actor accidentally accepted"
        );
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn canonical_atto_path_supports_ir_and_native_execution() {
    const SOURCE: &str = "fnc main():int{return (20+1)*2;}";
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "attolang-cranelift-ir-{}-{id}.atto",
        std::process::id()
    ));
    std::fs::write(&path, SOURCE).expect("create AttoLang IR fixture");
    let ir = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("ir")
        .arg(&path)
        .output()
        .expect("run AttoLang IR mode");
    assert!(
        ir.status.success(),
        "AttoLang IR mode rejected .atto: {}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(!ir.stdout.is_empty(), "IR output must not be empty");
    let guest = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("run AttoLang guest");
    std::fs::remove_file(path).expect("clean up AttoLang IR fixture");
    assert!(guest.status.success(), "{}", String::from_utf8_lossy(&guest.stderr));
    assert_eq!(guest.stdout, b"42\n");
}
