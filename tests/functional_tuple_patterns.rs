//! Execute real native JIT via public CLI. No host interpreter or AOT claim.
//! Exact 13 shared fixtures from blob 193249523cf18ef725f4900c30f3925c5bf8ce4b.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run_cases(compiler: &str, dir: &Temp, mode: &str, name: &str, source: &str) -> Output {
    let file = dir.0.join(format!("{name}.atto"));
    fs::write(&file, source).unwrap();
    let result = Command::new(compiler)
        .arg(mode)
        .arg(&file)
        .current_dir(&dir.0)
        .output()
        .expect("actual native compiler");
    assert_eq!(fs::read_to_string(&file).unwrap(), source);
    result
}
#[test]
fn shared_tuple_native_and_boundaries() {
    let compiler = env!("CARGO_BIN_EXE_orescranelift");
    let dir = Temp(
        std::env::temp_dir().join(format!(
            "atto-cranelift-tuples-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
    );
    fs::create_dir(&dir.0).unwrap();
    let positive = [
        (
            "flat-three",
            "pub fnc main(): int { return (5, 3, 5) |> |(a, b, c)| -> a + b + c; }\n",
            13,
        ),
        (
            "order-weighted",
            "pub fnc main(): int { return (2, 7, 11) |> |(a, b, c)| -> 2*a + 3*b + 4*c; }\n",
            69,
        ),
        (
            "discard-middle",
            "pub fnc main(): int { return (5, 99, 8) |> |(a, _, c)| -> a + c; }\n",
            13,
        ),
        (
            "discard-first",
            "pub fnc main(): int { return (99, 8) |> |(_, b)| -> b + 3; }\n",
            11,
        ),
        (
            "discard-last",
            "pub fnc main(): int { return (6, 99) |> |(a, _)| -> a + 5; }\n",
            11,
        ),
        (
            "discard-all",
            "pub fnc main(): int { return (5, 8) |> |(_, _)| -> 19; }\n",
            19,
        ),
        (
            "singleton",
            "pub fnc main(): int { return (9,) |> |(a,)| -> a; }\n",
            9,
        ),
        (
            "empty",
            "pub fnc main(): int { return () |> |()| -> 7; }\n",
            7,
        ),
        (
            "typed-leaves",
            "pub fnc main(): int { return (5, 8) |> |(a: int, b: int)| -> a + b; }\n",
            13,
        ),
        (
            "parenthesized-stage",
            "pub fnc main(): int { return ((5, 8) |> |(a, b)| -> a + b) + 1; }\n",
            14,
        ),
        (
            "nested-scalar-stage",
            "pub fnc main(): int { return (5, 8) |> |(a, b)| -> ((a, 2) |> |(x, y)| -> x*y) + b; }\n",
            18,
        ),
        (
            "signed-values",
            "pub fnc main(): int { return (-5, 8, 12) |> |(a, b, c)| -> a + b + c; }\n",
            15,
        ),
        (
            "maximum-arity",
            "pub fnc main(): int { return (1, 2, 3, 4, 5, 6, 7, 8) |> |(a,b,c,d,e,f,g,h)| -> a+b+c+d+e+f+g+h; }\n",
            36,
        ),
        (
            "shadow",
            "pub fnc main(): int { val a=19; return ((5,8) |> |(a,b)| -> a+b)+a; }\n",
            32,
        ),
        (
            "continuation",
            "fnc double(int v): int { return v*2; } pub fnc main(): int { return (5,8) |> |(a,b)| -> a+b |> double; }\n",
            26,
        ),
        (
            "repeated",
            "fnc f(int x): int { return x+1; } pub fnc main(): int { return (f(5),8) |> |(a,_)| -> a+a; }\n",
            12,
        ),
    ];
    for (id, source, want) in positive {
        let check = run_cases(compiler, &dir, "check", id, source);
        assert!(
            check.status.success() && check.stdout.is_empty() && check.stderr.is_empty(),
            "{id}: {:?}",
            check
        );
        let ir = run_cases(compiler, &dir, "ir", id, source);
        assert!(
            ir.status.success()
                && String::from_utf8_lossy(&ir.stdout).contains("linked function main")
                && ir.stderr.is_empty(),
            "{id}: {:?}",
            ir
        );
        let out = run_cases(compiler, &dir, "run", id, source);
        assert!(
            out.status.success() && out.stderr.is_empty(),
            "{id}: {:?}",
            out
        );
        assert_eq!(out.stdout, format!("{want}\n").into_bytes(), "{id}");
    }
    let negative = [
        ("shape", "1 |> |(a,b)| -> a+b", "E_PIPE_PATTERN_SHAPE"),
        ("arity", "(1,2,3) |> |(a,b)| -> a+b", "E_PIPE_ARITY"),
        ("duplicate", "(1,2) |> |(a,a)| -> a", "E_PIPE_DUP_PARAM"),
        ("singleton", "(1,) |> |(a)| -> a", "E_PIPE_PATTERN"),
        (
            "discard",
            "(1,2) |> |(a,_)| -> _",
            "E_LINK_UNBOUND_IDENTIFIER",
        ),
        (
            "typed-discard",
            "(1,2) |> |(_:int,b)| -> b",
            "E_PIPE_PATTERN",
        ),
        (
            "nested",
            "((1,2),3) |> |(a,b)| -> b",
            "E_LINK_UNSUPPORTED_TUPLE_VALUE",
        ),
        (
            "bool",
            "(true,2) |> |(a,b)| -> b",
            "E_LINK_PATTERN_ELEMENT_TYPE",
        ),
        ("escape", "(1,2)", "E_LINK_UNSUPPORTED_TUPLE_VALUE"),
        (
            "body-type",
            "(1,2) |> |(a,b)| -> a eq b",
            "E_LINK_PATTERN_RESULT_TYPE",
        ),
    ];
    for (id, expr, code) in negative {
        let source = format!("pub fnc main(): int {{ return {expr}; }}\n");
        for mode in ["check", "ir", "run"] {
            let out = run_cases(compiler, &dir, mode, &format!("negative-{id}"), &source);
            assert_eq!(out.status.code(), Some(1), "{id} {mode}: {:?}", out);
            assert!(
                out.stdout.is_empty() && String::from_utf8_lossy(&out.stderr).contains(code),
                "{id} {mode}: {:?}",
                out
            );
        }
    }
    for (id, expr) in [
        (
            "overflow-first",
            "(9223372036854775807+1,8) |> |(_,b)| -> b",
        ),
        ("overflow-last", "(5,9223372036854775807+1) |> |(a,_)| -> a"),
    ] {
        let source = format!("pub fnc main(): int {{ return {expr}; }}\n");
        let out = run_cases(compiler, &dir, "run", id, &source);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("i64 arithmetic overflow"),
            "{id}: {:?}",
            out
        );
    }
    let source = "pub fnc main(): int { return (5,8) |> |(a,b)| -> a+b; }\n";
    for mode in ["emit-object", "isolate"] {
        let out = run_cases(compiler, &dir, mode, "no-fallback", source);
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
    }
    println!(
        "CRANELIFT_TUPLE_NATIVE_PASS 13 shared + 3 additional positive + 30 rejection + 2 overflow + 2 no-fallback controls"
    );
}
