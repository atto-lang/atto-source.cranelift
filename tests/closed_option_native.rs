//! Actual native JIT and object-linked guests, not a host Error simulation.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "atto-closed-option-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).expect("exclusive test directory");
        Self(dir)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn invoke(mode: &str, path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}
fn source(init: &str, fallback: &str) -> String {
    format!(
        "pub fnc main(int n) -> int {{ val Option<int> x = {init}; return trap! x.unwrap() |> match<int> over on Ok(v): int -> v, on Err(): int -> {fallback}, end; }}"
    )
}
fn success(out: Output, expected: &[u8]) {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, expected);
    assert!(out.stderr.is_empty());
}
#[test]
fn actual_jit_closed_and_dynamic_option_match() {
    let ws = Workspace::new();
    let file = ws.0.join("guest.atto");
    for (init, fallback, cases) in [
        ("Some(n+1)", "42/0", vec![("41", "42\n"), ("-1", "0\n")]),
        ("None", "n+1", vec![("41", "42\n")]),
        (
            "n eq 0 ? Some(84/(n+2)) : None",
            "42/n",
            vec![("0", "42\n"), ("2", "21\n"), ("-2", "-21\n")],
        ),
        (
            "n eq 0 ? None : Some(42/n)",
            "40+n+2",
            vec![("0", "42\n"), ("2", "21\n")],
        ),
    ] {
        fs::write(&file, source(init, fallback)).unwrap();
        success(invoke("check", &file, &[]), b"");
        let ir = invoke("ir", &file, &[]);
        assert!(
            ir.status.success(),
            "{}",
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));
        for (arg, expected) in cases {
            success(invoke("run", &file, &[arg]), expected.as_bytes());
        }
    }
}
#[test]
fn errors_before_trap_and_in_selected_arm_are_not_caught() {
    let ws = Workspace::new();
    let file = ws.0.join("guest.atto");
    for (init, fallback, arg) in [
        ("Some(42/n)", "123", "0"),
        ("None", "42/n", "0"),
        ("n eq 0 ? Some(42/n) : None", "123", "0"),
        ("Some(n+1)", "123", "9223372036854775807"),
        ("None", "n+1", "9223372036854775807"),
    ] {
        fs::write(&file, source(init, fallback)).unwrap();
        success(invoke("check", &file, &[]), b"");
        let out = invoke("run", &file, &[arg]);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        let err = String::from_utf8_lossy(&out.stderr);
        let expected = if arg == "0" {
            "orescranelift: integer divisor zero\n"
        } else {
            "orescranelift: i64 arithmetic overflow\n"
        };
        assert_eq!(err, expected);
    }
}
#[test]
fn unsupported_sum_effect_escape_and_untyped_variants_fail_closed() {
    let ws = Workspace::new();
    let file = ws.0.join("guest.atto");
    let good = source("Some(n)", "42");
    for invalid in [
        good.replace("Option<int>", "Option<bool>"),
        good.replace("Some(n)", "Some(true)"),
        good.replace("Some(n)", "Some(x)"),
        good.replace("trap! x", "trap!x"),
        good.replace("trap! x", "trap ! x"),
        good.replace("|>", "| >"),
        good.replace("|>", "|/*gap*/>"),
        good.replace(") -> int", ") - > int"),
        good.replace("on Ok(v): int ->", "on Ok(v): int - >"),
        good.replace("on Err(): int ->", "on Err(): int -/*gap*/>"),
        good.replace("x.unwrap()", "x.expect()"),
        good.replace("x.unwrap()", "other.unwrap()"),
        good.replace("on Err()", "on Err(e)"),
        good.replace("on Ok(v): int -> v", "on Ok(v): int -> n"),
        good.replace("on Err(): int -> 42,", ""),
        good.replace("on Err(): int -> 42", "on Err(): int -> v"),
        good.replace("on Err(): int -> 42", "on Err(): int -> throw! 42"),
        good.replace("val Option<int> x", "val Option<int> n"),
        good.replace("Some(n)", "n eq 0 ? Some(n) : Some(1)"),
        format!("{good} fnc extra():int{{return 1;}}"),
    ] {
        fs::write(&file, &invalid).unwrap();
        for mode in ["check", "run", "ir", "emit-object", "emit-object-strict"] {
            let out = invoke(mode, &file, if mode == "run" { &["0"] } else { &[] });
            assert_eq!(out.status.code(), Some(1), "{mode}: {invalid}");
            assert!(out.stdout.is_empty(), "{mode}: emitted artifact");
        }
    }
    // Strict AOT has its own proof contract and must not silently fall back.
    fs::write(&file, good).unwrap();
    let strict = invoke("emit-object-strict", &file, &[]);
    assert_eq!(strict.status.code(), Some(1));
    assert!(strict.stdout.is_empty());
}
#[cfg(unix)]
#[test]
fn native_objects_execute_both_tags_and_preserve_output_on_fault() {
    let ws = Workspace::new();
    let file = ws.0.join("guest.atto");
    let obj = ws.0.join("guest.o");
    let harness = ws.0.join("harness.c");
    fs::write(&file, source("n eq 0 ? Some(42) : None", "84/(n-2)")).unwrap();
    let artifact = invoke("emit-object", &file, &[]);
    assert!(
        artifact.status.success(),
        "{}",
        String::from_utf8_lossy(&artifact.stderr)
    );
    assert!(!artifact.stdout.is_empty());
    fs::write(&obj, artifact.stdout).unwrap();
    fs::write(
        &harness,
        r#"
#include <stdint.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void) {
  int64_t out=777;
  assert(oreslang_scalar_entry(0,0,0,0,&out)==0 && out==42);
  assert(oreslang_scalar_entry(4,0,0,0,&out)==0 && out==42);
  out=777;
  assert(oreslang_scalar_entry(2,0,0,0,&out)==2 && out==777);
  assert(oreslang_scalar_entry(INT64_MIN,0,0,0,&out)==1 && out==777);
  return 0;
}
"#,
    )
    .unwrap();
    for opt in ["-O0", "-O2"] {
        let binary = ws.0.join(format!("guest{opt}"));
        let link = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Werror", opt])
            .arg(&harness)
            .arg(&obj)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            link.status.success(),
            "{}",
            String::from_utf8_lossy(&link.stderr)
        );
        assert!(Command::new(&binary).status().unwrap().success());
    }
}

#[test]
fn bounded_declaration_is_checked_not_stripped() {
    let ws = Workspace::new();
    let file = ws.0.join("guest.atto");
    let good = source("Some(n)", "42");
    for header in [
        "declare type Bounded = int where self >= 1 and self <= 42;",
        "declare type Bounded = int where self >= -9223372036854775808 and self <= 9223372036854775807;",
        "declare type Bounded = int where self >= 7 and self <= 7;",
    ] {
        fs::write(&file, format!("{header}\n{good}")).unwrap();
        success(invoke("run", &file, &["42"]), b"42\n");
    }
    for header in [
        "declare type Bounded = int where self >= 42 and self <= 1;",
        "declare type Bounded = int where self >= -9223372036854775809 and self <= 42;",
        "declare type Bounded = int where self >= 1 and self <= 9223372036854775808;",
        "declare type Bounded = int where self > = 1 and self <= 42;",
        "declare type Bounded = int where self >= 1 and self < = 42;",
        "declare type Bounded = bool where self >= 1 and self <= 42;",
        "declare type n = int where self >= 1 and self <= 42;",
        "declare type x = int where self >= 1 and self <= 42;",
        "declare type v = int where self >= 1 and self <= 42;",
        "declare type int = int where self >= 1 and self <= 42;",
        "declare type Bounded = int where self >= 1 and self <= 42; declare type Bounded = int where self >= 1 and self <= 42;",
    ] {
        fs::write(&file, format!("{header}\n{good}")).unwrap();
        for mode in ["check", "run", "ir", "emit-object", "emit-object-strict"] {
            let out = invoke(mode, &file, if mode == "run" { &["42"] } else { &[] });
            assert_eq!(out.status.code(), Some(1), "{mode}: {header}");
            assert!(out.stdout.is_empty(), "{mode}: emitted artifact");
        }
    }
}
