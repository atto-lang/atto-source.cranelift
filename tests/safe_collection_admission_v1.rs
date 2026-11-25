//! Regression on actual stage-0 Cranelift binary admission.
//! This does NOT prove any guest collection or Option implementation.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn execute(mode: &str, source: &str) -> std::process::Output {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-collection-cranelift-{}-{n}.atto",
        std::process::id()
    ));
    std::fs::write(&path, source).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn unavailable_collections_never_emit_clif_or_execute_i64_jit() {
    let names = [
        "Array",
        "FixedArray",
        "Vector",
        "List",
        "FixedList",
        "Slice",
        "Tuple",
        "Map",
        "Set",
        "ReadLoan",
        "WriteLoan",
    ];
    for name in names {
        let program = format!("pub fnc main():int{{return {name};}}");
        for mode in ["run", "ir"] {
            let result = execute(mode, &program);
            assert_eq!(result.status.code(), Some(1), "{mode}/{name}");
            assert!(
                result.stdout.is_empty(),
                "{mode}/{name} published native output"
            );
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.contains("E_SAFE_COLLECTION_ACCESS_UNSUPPORTED"),
                "{mode}/{name}: {diagnostic}"
            );
        }
    }
}

#[test]
fn tagged_option_and_ownership_operations_fail_with_exact_denial_in_both_modes() {
    // A nonzero process exit is not sufficient. Lexer/parser crashes or an
    // unrelated syntax error cannot count as a semantic borrow/Option proof.
    for (source, expected) in [
        (
            "pub fnc main():int{return Some(42);}",
            "typed Option/Result/trap is unsupported",
        ),
        (
            "pub fnc main():int{return None;}",
            "typed Option/Result/trap is unsupported",
        ),
        (
            "pub fnc main():int{return rt borrow value;}",
            "rt ownership operation requires checked ownership lowering",
        ),
        (
            "pub fnc main():int{return Array.of(42).get_copy(-1);}",
            "E_SAFE_COLLECTION_ACCESS_UNSUPPORTED",
        ),
        (
            "pub fnc main():int{return Array.of(42)[0];}",
            "E_SAFE_COLLECTION_ACCESS_UNSUPPORTED",
        ),
    ] {
        for mode in ["run", "ir"] {
            let result = execute(mode, source);
            assert_eq!(result.status.code(), Some(1), "{mode}/{source}");
            assert!(
                result.stdout.is_empty(),
                "{mode}: denied collection source produced stdout"
            );
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.contains(expected),
                "{mode}: expected {expected:?}; got {diagnostic:?}"
            );
        }
    }
}

#[test]
fn comments_mentioning_collections_do_not_change_scalar_guest_execution() {
    let source = "// Option<Array<int>> and ReadLoan are comments only\n        pub fnc main():int{return 42;}";
    let result = execute("run", source);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"42\n");
}
