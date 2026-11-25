//! Exact spec fixture -> normal Cranelift CLI -> actual native JIT execution.
//! The host test does not compute 83 or substitute for compiled guest code.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Source(PathBuf);
impl Source {
    fn new(source: &str) -> Self {
        for _ in 0..64 {
            let i = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-cranelift-pipeline-{}-{i}.atto",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    f.write_all(source.as_bytes()).unwrap();
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create guest source: {e}"),
            }
        }
        panic!("source namespace exhausted")
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn invoke(mode: &str, path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(path)
        .output()
        .expect("launch real Cranelift executable")
}
fn fixture(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/functional-pipelines-v1")
        .join(file)
}
#[test]
fn canonical_spec_native_pipeline_guests_are_executed_not_host_evaluated() {
    let positives = [
        ("block-body.atto", "83\n"),
        ("inline-arrow-body.atto", "83\n"),
        ("reverse-associative.atto", "42\n"),
        ("mixed-parenthesized.atto", "42\n"),
    ];
    for (file, expected) in positives {
        let src = fixture(file);
        let admission = invoke("check", &src);
        assert!(
            admission.status.success(),
            "check {file}: {}",
            String::from_utf8_lossy(&admission.stderr)
        );
        assert!(admission.stdout.is_empty());
        let ir = invoke("ir", &src);
        assert!(
            ir.status.success(),
            "ir {file}: {}",
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(
            String::from_utf8_lossy(&ir.stdout).contains("function"),
            "{file} emitted no native CLIF"
        );
        let executable = invoke("run", &src);
        assert!(
            executable.status.success(),
            "run {file}: {}",
            String::from_utf8_lossy(&executable.stderr)
        );
        assert_eq!(
            executable.stdout,
            expected.as_bytes(),
            "{file} wrong native result"
        );
        assert!(executable.stderr.is_empty(), "{file} unexpected stderr");
    }
}
#[test]
fn canonical_spec_negative_sources_fail_before_clif_or_jit() {
    for name in [
        "reject-return-type.atto",
        "reject-unterminated-body.atto",
        "reject-ambiguous-mixed-direction.atto",
        "reject-reverse-mixed-unparenthesized.atto",
    ] {
        let src = fixture(name);
        for mode in ["check", "ir", "run"] {
            let observed = invoke(mode, &src);
            assert_eq!(observed.status.code(), Some(1), "{mode} accepted {name}");
            assert!(
                observed.stdout.is_empty(),
                "{mode} emitted native code or guest stdout for {name}"
            );
            assert!(
                !observed.stderr.is_empty(),
                "{mode} has no diagnostic for {name}"
            );
        }
    }
}
#[test]
fn fail_closed_on_non_linear_loans_and_other_pre_ir_violations() {
    for src in [
        "fnc twice(int x): int -> x + x; pub fnc main(): int -> twice <| 21;",
        "fnc id(int x): int -> x; pub fnc main(): int { val x = 42; return 1; }",
        "fnc id(int x): int -> x; pub fnc main(): int { val a = 10; val b = 21; return b+a; }",
        "pub fnc main(): int { val x = 42; val x = 43; return x; }",
        "pub fnc main(): int { val a = b; val b = 42; return a; }",
        "pub fnc main(): int -> missing <| 42;",
        "pub fnc main(): int -> 42 <| 42;",
        "fnc double(int x): int -> x*2; pub fnc main(): int -> 20 |> double(5);",
        "fnc double(int x): int -> x*2; pub fnc main(): int -> double <| 20 |> double;",
        "fnc reverse(int a, int b): int -> b - a; pub fnc main(): int -> reverse(1, 2);",
        "fnc unsafe(int x): int -> x; pub fnc main(): int -> unsafe(42);",
        "pub fnc main(): int -> main();",
        "pub fnc main(): int -> 9223372036854775808;",
        "pub fnc main(): int -> true;",
        "pub fnc main(): int -> unsafe { return 42; };",
        "pub fnc main(): int -> (42 |> id; ",
    ] {
        let file = Source::new(src);
        for mode in ["check", "ir", "run"] {
            let observed = invoke(mode, &file.0);
            assert_eq!(
                observed.status.code(),
                Some(1),
                "{mode} incorrectly accepted {src}"
            );
            assert!(
                observed.stdout.is_empty(),
                "{mode} wrote unadmitted guest output for {src}"
            );
            assert!(!observed.stderr.is_empty(), "{mode} failed silently");
        }
    }
}
#[test]
fn signed_int64_boundary_compiles_into_native_clif_and_executes_without_wraparound() {
    let accepted = Source::new("pub fnc main(): int -> -9223372036854775808;");
    let checked = invoke("check", &accepted.0);
    assert!(
        checked.status.success(),
        "i64 minimum rejected: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let ir = invoke("ir", &accepted.0);
    assert!(
        ir.status.success(),
        "i64 minimum produced no valid IR: {}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));
    let run = invoke("run", &accepted.0);
    assert!(
        run.status.success(),
        "i64 minimum failed to execute: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"-9223372036854775808\n");
    assert!(run.stderr.is_empty());

    for invalid in [
        "pub fnc main(): int -> 9223372036854775808;",
        "pub fnc main(): int -> -9223372036854775809;",
    ] {
        let source = Source::new(invalid);
        for mode in ["check", "ir", "run"] {
            let rejected = invoke(mode, &source.0);
            assert_eq!(
                rejected.status.code(),
                Some(1),
                "out-of-range signed i64 accepted in {mode}"
            );
            assert!(
                rejected.stdout.is_empty(),
                "rejected source emitted IR or guest output"
            );
            assert!(!rejected.stderr.is_empty());
        }
    }
}

fn edge_fixture(file: &str) -> PathBuf {
    fixture(&format!("edge/{file}"))
}
#[test]
fn spec_edge_multiarg_reverse_and_signed_min_are_actual_jit_guests() {
    for (name, expected) in [
        ("two-argument-61.atto", "61\n"),
        ("preapplied-grouped-42.atto", "42\n"),
        ("signed-i64-min.atto", "-9223372036854775808\n"),
    ] {
        let source = edge_fixture(name);
        let checked = invoke("check", &source);
        assert!(
            checked.status.success(),
            "edge {name} check failed: {}",
            String::from_utf8_lossy(&checked.stderr)
        );
        assert!(
            checked.stdout.is_empty(),
            "{name}: check emitted guest stdout"
        );
        let clif = invoke("ir", &source);
        assert!(
            clif.status.success(),
            "edge {name} CLIF rejected: {}",
            String::from_utf8_lossy(&clif.stderr)
        );
        assert!(
            String::from_utf8_lossy(&clif.stdout).contains("function"),
            "{name} produced no native IR"
        );
        let run = invoke("run", &source);
        assert_eq!(
            run.status.code(),
            Some(0),
            "edge {name} JIT failed: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(run.stdout, expected.as_bytes(), "edge {name} native value");
        assert!(run.stderr.is_empty(), "edge {name} JIT stderr");
    }
}
#[test]
fn spec_edge_checked_overflow_traps_without_wraparound_or_guest_stdout() {
    let source = edge_fixture("checked-overflow.atto");
    assert!(invoke("check", &source).status.success());
    let clif = invoke("ir", &source);
    assert!(clif.status.success());
    assert!(String::from_utf8_lossy(&clif.stdout).contains("function"));
    let run = invoke("run", &source);
    assert_eq!(
        run.status.code(),
        Some(1),
        "overflow should be a checked native error"
    );
    assert!(
        run.stdout.is_empty(),
        "overflow may not return a wrapped integer"
    );
    assert!(String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"));
}
#[test]
fn spec_edge_negative_sources_fail_before_native_ir_or_execution() {
    for (name, expected) in [
        ("reject-i64-positive-overflow.atto", "E_PIPE_LITERAL"),
        ("reject-i64-negative-underflow.atto", "E_PIPE_LITERAL"),
        ("reject-reserved-unsafe.atto", "E_PIPE_NAME"),
    ] {
        let source = edge_fixture(name);
        for mode in ["check", "ir", "run"] {
            let rejected = invoke(mode, &source);
            assert_eq!(rejected.status.code(), Some(1), "{name}: {mode} accepted");
            assert!(
                rejected.stdout.is_empty(),
                "{name}: emitted guest code/result"
            );
            assert!(
                String::from_utf8_lossy(&rejected.stderr).contains(expected),
                "{name}: {mode} missed typed/authority error"
            );
        }
    }
}
#[test]
fn seven_spec_edge_guest_blobs_are_pinned_exactly() {
    for (name, expected) in [
        (
            "two-argument-61.atto",
            "4089c524db1d076593994ad65acb5ca7f90eea32",
        ),
        (
            "preapplied-grouped-42.atto",
            "9a89ada18865fead0d92a7de75415957fbb5f67f",
        ),
        (
            "signed-i64-min.atto",
            "bbc22347d7ccb0a5be1a7c2193f11c5f2f4805cc",
        ),
        (
            "checked-overflow.atto",
            "59bb890b54addc1659065ab55cd565771523bfce",
        ),
        (
            "reject-i64-positive-overflow.atto",
            "2e584ee01ee74a3a31e282cf0116957f9ff95bd1",
        ),
        (
            "reject-i64-negative-underflow.atto",
            "8b63b45e406db3e19aa793e0f7a627914623b9e0",
        ),
        (
            "reject-reserved-unsafe.atto",
            "b159def3fa26ca6023395e1c546b95c1b929fd14",
        ),
    ] {
        let file = edge_fixture(name);
        let out = Command::new("git")
            .arg("hash-object")
            .arg(&file)
            .output()
            .expect("git hash-object canonical edge file");
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap().trim(),
            expected,
            "{name}: canonical edge bytes changed"
        );
    }
}

#[test]
fn copied_spec_fixtures_have_exact_sha_pinned_git_blobs() {
    let pinned = [
        (
            "block-body.atto",
            "4a2eab34e1ba39709198afda19a71cfce67f4030",
        ),
        (
            "inline-arrow-body.atto",
            "ec25b1ce607afa7d30dd5315eace12ff42f0c4d6",
        ),
        (
            "reverse-associative.atto",
            "e71023cd0cc258c9650167e979c45ffb5162d9f7",
        ),
        (
            "mixed-parenthesized.atto",
            "0fcdce8f0b1381125d0148122441c81dfb2a0abf",
        ),
        (
            "reject-return-type.atto",
            "74b2f398a72b41407039ac68cdc44b23b7ece125",
        ),
        (
            "reject-unterminated-body.atto",
            "1d84e3f31fee46a73a6c111e406ebb883e5645b2",
        ),
        (
            "reject-ambiguous-mixed-direction.atto",
            "7e579225f25720da5255ecb47640831be6e3a1c7",
        ),
        (
            "reject-reverse-mixed-unparenthesized.atto",
            "2727d012e5482c407c2e642b3612144e67fad767",
        ),
    ];
    for (name, sha) in pinned {
        let source = fixture(name);
        let result = Command::new("git")
            .arg("hash-object")
            .arg(&source)
            .output()
            .expect("git hash-object fixture");
        assert!(result.status.success(), "{name}: git failed");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap().trim(),
            sha,
            "{name}: canonical source bytes changed"
        );
    }
}

#[test]
fn authority_and_task_words_are_not_scalar_function_parameter_or_local_names() {
    for name in ["sync", "task", "yield"] {
        let samples = [
            format!("fnc {name}(int x): int -> x; pub fnc main(): int -> 42 |> {name};"),
            format!("fnc f(int {name}): int -> 42; pub fnc main(): int -> f(1);"),
            format!("fnc id(int x): int -> x; pub fnc main(): int {{ val {name} = 42; return {name} |> id; }}"),
        ];
        for source in samples {
            let file = Source::new(&source);
            for mode in ["check", "ir", "run"] {
                let result = invoke(mode, &file.0);
                assert!(
                    !result.status.success(),
                    "{name}: reserved binding admitted in {mode}"
                );
                assert!(
                    result.stdout.is_empty(),
                    "{name}: rejected scalar source emitted guest artifact/result"
                );
                assert!(
                    String::from_utf8_lossy(&result.stderr).contains("E_PIPE_NAME"),
                    "{name}: expected a source-level authority rejection"
                );
            }
        }
    }
}

#[test]
fn standalone_scalar_task_bindings_remain_unadmitted() {
    for name in ["sync", "task", "yield"] {
        for source in [
            format!("fnc main({name}:int):int{{return {name};}}"),
            format!("pub fnc main(): int {{ val {name} = 42; return {name}; }}"),
        ] {
            let file = Source::new(&source);
            for mode in ["check", "ir", "run"] {
                let result = invoke(mode, &file.0);
                assert_eq!(result.status.code(), Some(1), "{name}: {mode}");
                assert!(result.stdout.is_empty());
                assert!(!result.stderr.is_empty());
            }
        }
    }
}
