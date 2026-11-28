//! Cross-feature regression: local constexpr/val, sequence and the independently
//! admitted pure-pipeline frontend must share a verified native JIT boundary.
//! These are *authored* tests until executed on this exact commit.
use std::io::Write;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn execute(mode: &str, source: &str, args: &[&str]) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-integration-{}-{id}.atto",
        std::process::id()
    ));
    let mut fixture = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("exclusive guest fixture; never follow an existing symlink");
    fixture
        .write_all(source.as_bytes())
        .expect("write guest bytes");
    drop(fixture);
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .expect("real native Cranelift CLI");
    std::fs::remove_file(&path).expect("remove guest fixture");
    output
}

fn admitted(source: &str, args: &[&str], expected: &[u8]) {
    for mode in ["check", "ir", "run"] {
        let runtime_args = if mode == "run" { args } else { &[][..] };
        let output = execute(mode, source, runtime_args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{mode}: unexpected diagnostic");
        if mode == "run" {
            assert_eq!(output.stdout, expected, "{mode}: guest value changed");
        } else if mode == "ir" {
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("function"),
                "IR mode must return real checked Cranelift IR"
            );
        } else {
            assert!(
                output.stdout.is_empty(),
                "check must not run or print guest"
            );
        }
    }
}

fn rejected(source: &str, expected_diag: &str) {
    for mode in ["check", "ir", "run"] {
        let output = execute(mode, source, &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{mode}: rejected source succeeded"
        );
        assert!(
            output.stdout.is_empty(),
            "{mode}: rejected source published IR/guest output"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected_diag),
            "{mode}: expected {expected_diag}; got {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn constant_seq_and_runtime_val_share_scalar_native_backend() {
    admitted(
        "fnc main(a:int):int{val int live=a+1;const expected: int=seq! (1,40+2);return seq! (live,expected);}",
        &["40"],
        b"42\n",
    );
    admitted(
        "fnc main():int{int answer=(> 1,2,42);val checked=seq! (answer-1,answer);return checked;}",
        &[],
        b"42\n",
    );
    admitted(
        "fnc main(a:int):int{val first=seq! (1,a+2);val second=seq! (first,42);return second;}",
        &["40"],
        b"42\n",
    );
}

#[test]
fn const_seq_folds_only_proven_values_and_preserves_runtime_faults() {
    rejected(
        "fnc main():int{const x=seq! (9223372036854775807+1,42);return x;}",
        "E_CONST_OVERFLOW",
    );
    admitted("fnc main(a:int):int{const x=seq! (a+1,42);return x;}", &["1"], b"42\n");
    // Local const is eagerly evaluated once, not necessarily constexpr (spec #307).
    for (source, args, diagnostic) in [
        ("fnc main(a:int):int{const x=seq! (a+1,42);return x;}", vec!["9223372036854775807"], "overflow"),
        ("fnc main():int{const x=seq! (Integer.rem(4,0),42);return x;}", vec![], "zero"),
    ] {
        for mode in ["check","ir"] {
            let out=execute(mode,source,&[]);
            assert!(out.status.success(),"{mode}: {}",String::from_utf8_lossy(&out.stderr));
        }
        let out=execute("run",source,&args);
        assert_eq!(out.status.code(),Some(1),"{source}");
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains(diagnostic),"{source}: {}",String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn unobserved_runtime_initializer_seq_must_not_erase_traps() {
    let source = "fnc main(a:int):int{val ignored=seq! (a+1,42);return 42;}";
    let ir = execute("ir", source, &[]);
    assert_eq!(
        ir.status.code(),
        Some(0),
        "runtime initializer should admit IR"
    );
    assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));

    let safe = execute("run", source, &["1"]);
    assert_eq!(safe.status.code(), Some(0));
    assert_eq!(safe.stdout, b"42\n");

    let trapped = execute("run", source, &["9223372036854775807"]);
    assert_eq!(
        trapped.status.code(),
        Some(1),
        "discarded initializer overflow"
    );
    assert!(
        trapped.stdout.is_empty(),
        "no result published after a trap"
    );
    assert!(String::from_utf8_lossy(&trapped.stderr).contains("overflow"));
}

#[test]
fn pipeline_frontend_remains_native_and_mixed_profiles_fail_closed() {
    let pipelines = include_str!("fixtures/functional-pipelines-v1/inline-arrow-body.atto");
    admitted(pipelines, &[], b"83\n");

    // A single-guest mixed pipeline + sequence requires a shared typed IR.
    // Until then it may not be guessed as a pure function or host-evaluated.
    let mixed = "fnc double(int x):int{return x*2;} fnc main():int{return seq! (1,21) |> double;}";
    for mode in ["check", "ir", "run"] {
        let output = execute(mode, mixed, &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{mode}: unsupported mix admitted"
        );
        assert!(
            output.stdout.is_empty(),
            "{mode}: unverified mixed source published output"
        );
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "));
    }
}

#[test]
fn pure_string_key_pair_scalar_projection_runs_through_native_cli() {
    for source in [
        r#"fnc main():int{return Pair.of("count",42).second;}"#,
        r#"fnc main():int{return Pair.of<int>("count",42).second;}"#,
        r#"fnc main():int{return Pair.of<string,int>("count",42).second;}"#,
        r#"fnc main():int{return (Pair.of("first-name",20).second+1)*2;}"#,
        r#"fnc main():int{return Pair.of("",42).second;}"#,
    ] {
        admitted(source, &[], b"42\n");
    }
}

#[test]
fn pair_key_and_owned_product_boundaries_are_fail_closed() {
    for (source, diagnostic) in [
        (
            "fnc main():int{return Pair.of(42,5).second;}",
            "E_PAIR_KEY_TYPE",
        ),
        (
            "fnc main():int{return Pair.of(true,5).second;}",
            "E_PAIR_KEY_TYPE",
        ),
        (
            "fnc main():int{return Pair.of<int,int>(42,5).second;}",
            "E_PAIR_KEY_TYPE",
        ),
        (
            r#"fnc main():int{return Pair.of("count",42);}"#,
            "E_PAIR_ESCAPE_UNSUPPORTED",
        ),
        (
            r#"fnc main():int{return Pair.of("count",42).first;}"#,
            "E_PAIR_PROJECTION_UNSUPPORTED",
        ),
        (
            r#"fnc main(Pair:int):int{return Pair.of("count",42).second;}"#,
            "E_PAIR_SCOPE_UNSUPPORTED",
        ),
        (
            r#"fnc main():int{return Pair.of("count",9223372036854775808).second;}"#,
            "i64 literal out of range",
        ),
        (
            r#"fnc main():int{return Pair.of("count",missing).second;}"#,
            "unbound or unsupported identifier",
        ),
    ] {
        rejected(source, diagnostic);
    }
}

#[test]
fn checked_pair_projection_preserves_parameter_arithmetic_and_traps() {
    admitted(
        r#"fnc main(a:int):int{return Pair.of<int>("key",a*2).second;}"#,
        &["21"],
        b"42\n",
    );
    admitted(
        r#"fnc main(a:int):int{return Pair.of<int>("outer",Pair.of("inner",a+1).second).second;}"#,
        &["41"],
        b"42\n",
    );
    rejected(
        r#"fnc main(a:int):int{val n=a;return Pair.of("key",n).second;}"#,
        "E_PAIR_VALUE_UNSUPPORTED",
    );
    let source = r#"fnc main(a:int):int{return Pair.of("key",a+1).second;}"#;
    let output = execute("run", source, &["9223372036854775807"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("overflow"));
}

#[test]
fn dynamic_entry_native_projection_evaluates_both_operands() {
    admitted(
        "fnc main(a:int):int{return Entry.of<int,int>(a+1,a*2).value;}",
        &["21"],
        b"42\n",
    );
    admitted(
        "fnc main(a:int):int{return Entry.of(a+1,a*2).key;}",
        &["41"],
        b"42\n",
    );
    for source in [
        "fnc main(a:int):int{return Entry.of(42,a+1).key;}",
        "fnc main(a:int):int{return Entry.of(a+1,42).value;}",
    ] {
        let output = execute("run", source, &["9223372036854775807"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("overflow"));
    }
}

#[test]
fn entry_projection_rejects_unproved_operands_escape_and_shadowing() {
    for (source, diagnostic) in [
        (
            "fnc main():int{return Entry.of(1,2);}",
            "E_ENTRY_ESCAPE_UNSUPPORTED",
        ),
        (
            "fnc main():int{return Entry.of(1,2).second;}",
            "E_ENTRY_PROJECTION_UNSUPPORTED",
        ),
        (
            "fnc main(Entry:int):int{return Entry.of(1,2).key;}",
            "E_SAFE_COLLECTION_ACCESS_UNSUPPORTED",
        ),
        (
            "fnc main(a:int):int{val n=a;return Entry.of(42,n).key;}",
            "E_ENTRY_VALUE_UNSUPPORTED",
        ),
        (
            "fnc main():int{return Entry.of(42,missing).key;}",
            "unbound or unsupported identifier",
        ),
    ] {
        rejected(source, diagnostic);
        let output = execute("emit-object", source, &[]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn entry_projection_preserves_first_fault_for_both_fields() {
    // Both operands trap with different native error statuses. Selecting an
    // earlier slot must not erase the second evaluation; selecting the later
    // slot must not move its evaluation ahead of the first.
    for (source, diagnostic) in [
        (
            "fnc main(a:int):int{return Entry.of(a/0,9223372036854775807+a).key;}",
            "integer divisor zero",
        ),
        (
            "fnc main(a:int):int{return Entry.of(a/0,9223372036854775807+a).value;}",
            "integer divisor zero",
        ),
        (
            "fnc main(a:int):int{return Entry.of(9223372036854775807+a,a/0).key;}",
            "i64 arithmetic overflow",
        ),
        (
            "fnc main(a:int):int{return Entry.of(9223372036854775807+a,a/0).value;}",
            "i64 arithmetic overflow",
        ),
        (
            "fnc main(a:int):int{return Entry.of(42,a/0).key;}",
            "integer divisor zero",
        ),
        (
            "fnc main(a:int):int{return Entry.of(a/0,42).value;}",
            "integer divisor zero",
        ),
    ] {
        let output = execute("run", source, &["1"]);
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(
            output.stdout.is_empty(),
            "{source}: output published after fault"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr.trim(),
            format!("orescranelift: {diagnostic}"),
            "{source}"
        );
    }
}
