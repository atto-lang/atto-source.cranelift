use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn directory() -> std::path::PathBuf {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-aot-admission-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

fn execute(mode: &str, source: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(source)
        .output()
        .unwrap()
}

#[test]
fn full_aot_requests_cannot_fall_back_to_jit_or_create_artifacts() {
    let dir = directory();
    let source = dir.join("valid.ores");
    std::fs::write(&source, "pub fnc main():int{return 42;}\n").unwrap();

    for mode in ["aot", "full-aot"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&source)
            .arg(dir.join("output"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{mode}");
        assert!(output.stdout.is_empty(), "{mode}: unexpected guest result");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("full AOT object/executable emission is not implemented")
                && diagnostic.contains("refusing JIT fallback"),
            "{mode}: {diagnostic}"
        );
        assert!(!diagnostic.contains("panic"), "{mode}: compiler panic");
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_file(source).unwrap();
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn unsupported_state_and_actor_source_never_emits_ir_or_executes() {
    let dir = directory();
    for (index, (source_text, expected)) in [
        (
            "@Inject(MutableStateContainer())\ndefine class Counter as\nend",
            "STATE_INJECTION_UNSUPPORTED:",
        ),
        (
            "@StateBacked(MutableStateContainer(scope = #instance))\ndefine class Counter as\nend",
            "STATE_INJECTION_UNSUPPORTED:",
        ),
        (
            "@MutableStateContainer()\ndefine class Counter as\nend",
            "STATE_INJECTION_UNSUPPORTED:",
        ),
        (
            "define state container Counter as\nend",
            "DECL_KIND_UNSUPPORTED:",
        ),
        (
            "define pub state container Counter as\nend",
            "DECL_KIND_UNSUPPORTED:",
        ),
        (
            "define virtual actor Counter as\nend",
            "state containers and actor declarations require typed ownership",
        ),
        ("define pub actor Counter as\nend", "DECL_KIND_UNSUPPORTED:"),
        (
            "fnc main():int{return rt borrow 1;}",
            "rt ownership operation requires checked ownership lowering",
        ),
        (
            "fnc main():int{return rt mut share 1;}",
            "native rt mut share requires verified SharedMutex<T> capability",
        ),
    ]
    .iter()
    .enumerate()
    {
        let path = dir.join(format!("rejected-{index}.ores"));
        std::fs::write(&path, source_text).unwrap();
        for mode in ["run", "ir"] {
            let result = execute(mode, &path);
            assert_eq!(result.status.code(), Some(1), "{mode}: {source_text}");
            assert!(
                result.stdout.is_empty(),
                "{mode}: emitted guest stdout or IR"
            );
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(stderr.contains(expected), "{mode}: {stderr}");
            assert!(!stderr.contains("panic"), "{mode}: crashed: {stderr}");
        }
        std::fs::remove_file(path).unwrap();
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn annotations_in_comments_do_not_reject_valid_scalar_guest() {
    let dir = directory();
    let source = dir.join("comment-control.ores");
    std::fs::write(
        &source,
        "// @Inject(MutableStateContainer())\n// define state container Hidden as end\npub fnc main():int{return 40+2;}\n",
    )
    .unwrap();
    let result = execute("run", &source);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    std::fs::remove_file(source).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
