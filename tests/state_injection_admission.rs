//! Real CLI admission gates for unsupported injected mutable state.
//! A rejection is not a claim that the Cranelift backend executes containers.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run_source(source: &str, mode: &str) -> std::process::Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ores-state-injection-{}-{sequence}.ores",
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
fn injection_capabilities_fail_closed_in_run_and_ir() {
    let programs = [
        "@Inject(MutableStateContainer())\ndefine class Foo as\nstate {}\nend\n",
        "@Inject(MutableStateContainer(scope = #instance))\ndefine class Foo as\nend\n",
        "@StateBacked(MutableStateContainer(scope = #shared))\ndefine class Foo as\nend\n",
        "@MutableStateContainer()\ndefine singleton Foo as\nstate {}\nend\n",
        "@StateContainer()\ndefine actor Foo as\nstate {}\nend\n",
        "@Inject(SessionState)\ndefine class Foo as\nend\n",
        "@Inject\ndefine fnc main():int{return 7;}\n",
    ];
    for program in programs {
        for mode in ["run", "ir"] {
            let result = run_source(program, mode);
            assert!(!result.status.success(), "{mode} accepted {program}");
            assert!(
                result.stdout.is_empty(),
                "{mode} emitted guest output or IR for rejected {program}"
            );
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                stderr.contains("STATE_INJECTION_UNSUPPORTED"),
                "{mode} incorrect diagnostic for {program}: {stderr}"
            );
        }
    }
}

#[test]
fn unknown_annotations_are_explicitly_unsupported() {
    for mode in ["run", "ir"] {
        let result = run_source("@AnotherAnnotation\ndefine fnc main():int{return 7;}", mode);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("ANNOTATION_UNSUPPORTED")
        );
    }
}

#[test]
fn comments_cannot_trigger_injection_diagnostics_or_break_scalar_main() {
    for mode in ["run", "ir"] {
        let result = run_source(
            "// @Inject(MutableStateContainer()) is not executable source\n             define fnc main():int{return 7;}\n",
            mode,
        );
        assert!(
            result.status.success(),
            "{mode} rejected comment-only annotation: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!result.stdout.is_empty(), "{mode} failed to run/emit scalar control");
    }
}
