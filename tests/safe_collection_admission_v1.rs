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
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    drop(file);
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
        for mode in ["check", "run", "ir"] {
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
        for mode in ["check", "run", "ir"] {
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

#[test]
fn both_entry_ascriptions_and_map_mutation_reject_before_clif_or_jit() {
    let samples = [
        r#"let entry1 = ("foo" => 5) as Entry<string, int>;
           let entry2 = "foo" => 5 (as Entry<string, int>);
           pub fnc main(): int {
             let mut Map<string, int> x = {"foo" => 5};
             x.set("foo", 6); x["foo"] = 3;
             x.add(entry1); x.add(entry2);
             return x.get("foo").unwrap();
           }"#,
        r#"pub fnc main(): int {
              let e = "foo" => 5 (as Entry<string, int>);
              return 0;
           }"#,
        r#"pub fnc main(): int { return 1 => 2; }"#,
    ];
    for source in samples {
        for mode in ["check", "run", "ir"] {
            let output = execute(mode, source);
            assert_eq!(output.status.code(), Some(1), "{mode}: {source}");
            assert!(output.stdout.is_empty(), "unsupported guest emitted output");
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(diagnostic.contains("E_SAFE_COLLECTION_ACCESS_UNSUPPORTED")
                    || diagnostic.contains("E_UNSUPPORTED_MAP_ENTRY"),
                "{mode}: missing intentional typed feature denial: {diagnostic}");
        }
    }
}

#[test]
fn closed_record_and_nominal_struct_bracket_writes_never_get_scalar_authority() {
    for (source, diagnostic) in [
        (r#"pub fnc main(): int {
            let mut Record r = {foo: 1}; r["foo"] = 3; return r["foo"];
        }"#, "E_SAFE_COLLECTION_ACCESS_UNSUPPORTED"),
        (r#"define struct Point as int foo; end
            pub fnc main(): int {
              let mut Point p = Point{foo: 1}; p["foo"] = 3; return 0;
            }"#, "E_UNSUPPORTED_INDEX_WRITE"),
    ] {
        for mode in ["check", "run", "ir"] {
            let output = execute(mode, source);
            assert_eq!(output.status.code(), Some(1), "{mode}: {source}");
            assert!(output.stdout.is_empty(), "{mode}: unsupported guest emitted output");
            assert!(String::from_utf8_lossy(&output.stderr).contains(diagnostic),
                "{mode}: {}", String::from_utf8_lossy(&output.stderr));
        }
    }
}

#[test]
fn commented_entry_record_and_brackets_cannot_spoof_real_tokens() {
    let source = r#"// let e = "foo" => 5 (as Entry<string,int>); r["foo"] = 3;
        /* Map<string,int> Record Entry => [x] nested /* Entry => */ */
        pub fnc main(): int { return 42; }"#;
    let output = execute("run", source);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"42\n");
}
