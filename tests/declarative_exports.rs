//! Compiler-backed declarative export checks for the stage-0 scalar profile.
//! Do not import private binary modules or count parser-only tests as guest proof.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);
const MAIN: &str = "fnc main():int{return 42;}";

fn invoke(mode: &str, source: &str) -> std::process::Output {
    let sequence = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-declarative-export-{}-{sequence}.atto",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write source fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output();
    std::fs::remove_file(&path).expect("remove source fixture");
    result.expect("launch native Cranelift compiler")
}

#[test]
fn exports_before_or_after_main_compile_and_execute() {
    for source in [
        format!("export pub main;\n{MAIN}"),
        format!("{MAIN}\nexport pub fnc main;"),
        format!("export pub main;\nexport pub fnc main;\n{MAIN}"),
        format!("export pub main;\n{MAIN}\nexport pub main;"),
    ] {
        let run = invoke("run", &source);
        assert_eq!(
            run.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(run.stdout.as_slice(), b"42\n");
        assert!(run.stderr.is_empty());

        let ir = invoke("ir", &source);
        assert_eq!(
            ir.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(
            !ir.stdout.is_empty(),
            "successful source emitted no Cranelift IR"
        );
        assert!(ir.stderr.is_empty());
    }
}

#[test]
fn unsupported_and_conflicting_exports_fail_before_ir_or_guest_stdout() {
    for (source, diagnostic) in [
        (
            format!("export pub[pkg] fnc main;\n{MAIN}"),
            "EXPORT_PACKAGE_LINKER_UNSUPPORTED",
        ),
        (
            format!("export pub class Foo;\n{MAIN}"),
            "EXPORT_KIND_UNSUPPORTED",
        ),
        (format!("export pub Missing;\n{MAIN}"), "unexpected token"),
        (
            format!("export pub main;\nexport pub[pkg] fnc main;\n{MAIN}"),
            "EXPORT_PACKAGE_LINKER_UNSUPPORTED",
        ),
        (
            format!("export pub main;\n{MAIN}\nexport pub class main;"),
            "EXPORT_KIND_UNSUPPORTED",
        ),
        (
            format!("export pub[friend] fnc main;\n{MAIN}"),
            "unexpected token",
        ),
    ] {
        for mode in ["run", "ir"] {
            let out = invoke(mode, &source);
            assert_eq!(
                out.status.code(),
                Some(1),
                "{mode} must reject with compiler exit 1"
            );
            assert!(
                out.stdout.is_empty(),
                "{mode} emitted stdout/IR on rejection"
            );
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.starts_with("orescranelift: "),
                "wrong diagnostic origin: {stderr}"
            );
            assert!(
                stderr.contains(diagnostic),
                "{mode}: expected {diagnostic}, got {stderr}"
            );
            assert!(
                !stderr.contains("panicked at"),
                "compiler panicked: {stderr}"
            );
        }
    }
}
