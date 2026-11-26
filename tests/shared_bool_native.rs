#![cfg(unix)]
// Actual compiler CLI and native JIT execution, not host evaluation.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Source(PathBuf);
impl Source {
    fn new(text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "atto-bool-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.atto"), text).unwrap();
        Self(path)
    }
    fn invoke(&self, mode: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(self.0.join("main.atto"))
            .output()
            .unwrap()
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn succeeds(text: &str, expected: &[u8]) {
    let source = Source::new(text);
    let check = source.invoke("check");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let ir = source.invoke("ir");
    assert!(
        ir.status.success(),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    let clif = String::from_utf8(ir.stdout).unwrap();
    assert!(clif.contains("call ") && clif.contains("brif "), "{clif}");
    let run = source.invoke("run");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, expected);
    assert!(run.stderr.is_empty());
}
fn rejects(text: &str, diagnostic: &str) {
    let source = Source::new(text);
    for mode in ["check", "ir", "run"] {
        let run = source.invoke(mode);
        assert_eq!(run.status.code(), Some(1), "{mode}");
        assert!(run.stdout.is_empty(), "{mode}");
        assert!(
            String::from_utf8_lossy(&run.stderr).contains(diagnostic),
            "{mode}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}
#[test]
fn exact_shared_guest_runs_native() {
    succeeds(
        include_str!("fixtures/core-types-v1/bool-call-composition-positive.atto"),
        b"0\n",
    );
}
#[test]
fn typed_parameters_and_keyword_precedence_execute() {
    succeeds(
        "fnc pick(bool a, b: bool):bool { return a or b and false xor true; } pub fnc main():int { if pick(false, true) then return 42; fi return 7; }",
        b"42\n",
    );
}
#[test]
fn short_circuit_skips_checked_failure_but_xor_is_eager() {
    let helper = "fnc risky(int n):bool{return 9223372036854775807+n eq 0;}";
    succeeds(
        &format!(
            "{helper} pub fnc main():int{{ if false and risky(1) then return 7; fi if true or risky(1) then return 42; fi return 0; }}"
        ),
        b"42\n",
    );
    for expr in [
        "true and risky(1)",
        "false or risky(1)",
        "false xor risky(1)",
        "true xor risky(1)",
    ] {
        let source = Source::new(&format!(
            "{helper} pub fnc main():int{{if {expr} then return 42; fi return 7;}}"
        ));
        assert!(source.invoke("check").status.success());
        let run = source.invoke("run");
        assert_eq!(run.status.code(), Some(1), "{expr}");
        assert!(run.stdout.is_empty());
        assert!(String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"));
    }
}
#[test]
fn typed_graph_rejects_mixed_types_including_dormant_helpers() {
    for (src, expected) in [
        (
            "fnc f(bool x):bool{return x;} pub fnc main():int{if f(1) then return 42; fi return 0;}",
            "E_LINK_ARGUMENT_TYPE",
        ),
        (
            "fnc f(int x):bool{return x;} pub fnc main():int{return 42;}",
            "E_LINK_RETURN_TYPE",
        ),
        (
            "fnc f(bool x):int{return x+1;} pub fnc main():int{return 42;}",
            "E_LINK_INTEGER_OPERANDS",
        ),
        (
            "fnc f(bool x):bool{return x eq 1;} pub fnc main():int{return 42;}",
            "E_LINK_EQUALITY_TYPES",
        ),
        (
            "fnc f(bool x):bool{return x and 1;} pub fnc main():int{return 42;}",
            "E_LINK_BOOLEAN_OPERANDS",
        ),
        (
            "fnc f(bool x):bool{return x or 1;} pub fnc main():int{return 42;}",
            "E_LINK_BOOLEAN_OPERANDS",
        ),
        (
            "fnc f(bool x):bool{return x xor 1;} pub fnc main():int{return 42;}",
            "E_LINK_BOOLEAN_OPERANDS",
        ),
        (
            "fnc f(int x):int{return x;} pub fnc main():int{if f(1) then return 42; fi return 0;}",
            "E_LINK_BOOLEAN_CONDITION",
        ),
        (
            "fnc f(bool x):int{if x then return true; fi return 0;} pub fnc main():int{return 42;}",
            "E_LINK_BRANCH_TYPES",
        ),
        (
            "fnc f(bool x):bool{return x;} pub fnc main():bool{return f(true);}",
            "E_LINK_MAIN_TYPE",
        ),
    ] {
        rejects(src, expected);
    }
}

#[test]
fn named_scalar_reductions_use_typed_native_boolean_operations() {
    succeeds(
        "fnc gate(bool a, bool b):bool{return Xor(a,b) && !And(a,b) && Or(a,b);} pub fnc main():int{if gate(true,false) && gate(false,true) && !gate(true,true) && !gate(false,false) && Xor(true,true,true) && !Xor(true,true) && And(true,true,true,true) && !Or(false,false,false,false) then return 42; fi return 7;}",
        b"42\n",
    );
    let single = Source::new(
        "pub fnc main():int{if And(true,true) && Xor(false,true) then return 42; fi return 7;}",
    );
    assert!(single.invoke("check").status.success());
    let run = single.invoke("run");
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(run.stdout, b"42\n");
    assert!(run.stderr.is_empty());
}

#[test]
fn named_scalar_short_circuit_and_ordinary_resolution() {
    let helper = "fnc risky(int n):bool{return 9223372036854775807+n eq 0;}";
    succeeds(
        &format!(
            "{helper} pub fnc main():int{{if !And(false,risky(1),true) && Or(true,risky(1),false) then return 42; fi return 7;}}"
        ),
        b"42\n",
    );
    for expr in [
        "And(true,risky(1))",
        "Or(false,risky(1))",
        "Xor(false,risky(1))",
    ] {
        let source = Source::new(&format!(
            "{helper} pub fnc main():int{{if {expr} then return 42; fi return 7;}}"
        ));
        assert!(source.invoke("check").status.success());
        let run = source.invoke("run");
        assert_eq!(run.status.code(), Some(1), "{expr}");
        assert!(run.stdout.is_empty());
        assert!(String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"));
    }
    // User-defined And is an ordinary eager function, including a
    // non-Boolean signature; its arguments are not rewritten as gates.
    let shadow =
        Source::new("fnc And(int a,int b):int{return a+b;} pub fnc main():int{return And(20,22);}");
    let run = shadow.invoke("run");
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(run.stdout, b"42\n");
    assert!(run.stderr.is_empty());
    let eager = Source::new(&format!(
        "{helper} fnc And(bool a,bool b):bool{{return a;}} pub fnc main():int{{if And(false,risky(1)) then return 42; fi return 7;}}"
    ));
    assert!(eager.invoke("check").status.success());
    let run = eager.invoke("run");
    assert_eq!(run.status.code(), Some(1));
    assert!(run.stdout.is_empty());
    assert!(String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"));
}

#[test]
fn named_scalar_reductions_reject_arity_types_and_shadowed_local() {
    for name in ["And", "Or", "Xor"] {
        for (args, diagnostic) in [
            ("", "E_LINK_BOOLEAN_ARITY"),
            ("true", "E_LINK_BOOLEAN_ARITY"),
            ("true,1", "E_LINK_BOOLEAN_OPERANDS"),
            ("false,1", "E_LINK_BOOLEAN_OPERANDS"),
        ] {
            rejects(
                &format!(
                    "fnc bad():bool{{return {name}({args});}} pub fnc main():int{{return 42;}}"
                ),
                diagnostic,
            );
        }
    }
    rejects(
        "fnc bad(bool And):bool{return And(true,false);} pub fnc main():int{return 42;}",
        "E_LINK_BOOLEAN_SHADOW",
    );
}

#[test]
fn lexical_noncallable_gate_name_does_not_fall_through_to_declared_function() {
    for name in ["And", "Or", "Xor"] {
        rejects(
            &format!(
                "fnc {name}(bool a,bool b):bool{{return a;}} fnc bad(bool {name}):bool{{return {name}(true,false);}} pub fnc main():int{{return 42;}}"
            ),
            "E_LINK_BOOLEAN_SHADOW",
        );
    }
}
