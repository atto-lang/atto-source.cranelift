//! Real stage-0 .atto -> verified Cranelift IR -> native JIT sequence tests.
//! These tests are authored regression gates, not evidence of successful CI.
use std::io::Write;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn guest(mode: &str, source: &str) -> Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-seq-{}-{id}.atto",
        std::process::id()
    ));
    // Fail rather than following any pre-existing symlink or clobbering a
    // previous test's guest. The compiler separately enforces O_NOFOLLOW.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create unique, non-symlinked seq source");
    file.write_all(source.as_bytes()).expect("write guest");
    drop(file);
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("invoke independent native Cranelift compiler");
    std::fs::remove_file(path).expect("delete guest source");
    result
}

fn program(expr: &str) -> String {
    format!("pub fnc main(): int {{ return {expr}; }}\n")
}

#[test]
fn source_ordered_seq_and_scoped_shorthand_generate_actual_native_jit_values() {
    for expression in [
        "seq! (42)",
        "seq! (1, 2, 42)",
        "seq 1, 2, 42",
        "seq! 1, 2, 42",
        "seq!(1, 2, 42)",
        "(> 1, 2, 42)",
        "seq! (1, seq! (2, 42))",
        "seq! (1, (> 2, 42))",
        "(> 1, seq! (2, 42))",
        "seq! (1, 2) + 40",
        "seq! (1, Integer.bitAnd(7,3), 42)",
        "seq! (1, Integer.rem(8,3), 42)",
        "seq! (1, 7 >> 8, 42)",
        "seq! (1, /* comma,is inert */ 42)",
        "seq! (1, // comma, in comment\n 42)",
        "seq! (1, 2) as int + 40",
    ] {
        let source = program(expression);
        let ir = guest("ir", &source);
        assert!(
            ir.status.success(),
            "{expression}: IR failure: {}",
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(!ir.stdout.is_empty(), "{expression}: IR emitted nothing");
        let run = guest("run", &source);
        assert!(
            run.status.success(),
            "{expression}: JIT failure: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(run.stdout, b"42\n", "{expression}: wrong guest bytes");
        assert!(
            run.stderr.is_empty(),
            "{expression}: unexpected diagnostics"
        );
    }
}

#[test]
fn discarded_prefix_must_still_trap_in_generated_native_code() {
    for (expression, expected_diagnostic) in [
        ("seq! (9223372036854775807 + 1, 42)", "overflow"),
        ("seq! 9223372036854775807 + 1, 42", "overflow"),
        ("(> 9223372036854775807 + 1, 42)", "overflow"),
        ("seq! (1, seq! (9223372036854775807 + 1, 42))", "overflow"),
        ("seq! (Integer.rem(8, 0), 42)", "divisor zero"),
        ("(> Integer.mod(8, 0), 42)", "divisor zero"),
        ("seq! (Integer.pow(2, 63), 42)", "overflow"),
    ] {
        let source = program(expression);
        let ir = guest("ir", &source);
        assert!(
            ir.status.success(),
            "{expression}: failed before checked IR: {}",
            String::from_utf8_lossy(&ir.stderr)
        );
        let run = guest("run", &source);
        assert_eq!(
            run.status.code(),
            Some(1),
            "{expression}: wrong trap exit code"
        );
        assert!(
            run.stdout.is_empty(),
            "{expression}: emitted final value after a trap"
        );
        let diagnostic = String::from_utf8_lossy(&run.stderr);
        assert!(
            diagnostic.starts_with("orescranelift: ") && diagnostic.contains(expected_diagnostic),
            "{expression}: missing compiler-owned trap diagnostic: {diagnostic}"
        );
    }
}

#[test]
fn malformed_seq_and_unlowered_effects_fail_before_ir_or_jit() {
    let mut invalid: Vec<String> = [
        "seq! ()",
        "(>)",
        "seq! (1,)",
        "(> 1,)",
        "seq! (,1)",
        "seq! (1,,2)",
        "seq! (1;2)",
        "seq! (1 2)",
        "(1,2)",
        "seq 1,",
        "seq! 1,",
        "seq(1,42)",
        "seq (1,42)",
        "seq !(1,42)",
        "seq! (1, seq! ())",
        "seq! (1, rt take x)",
        "seq! (1, \"a,b\")",
        "seq! (1, await x)",
    ]
    .iter()
    .map(|value| program(value))
    .collect();
    invalid.push(program(&format!(
        "seq! ({})",
        (0..17).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
    )));
    for source in invalid {
        for mode in ["ir", "run"] {
            let output = guest(mode, &source);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{mode}: unsafe sequence admitted: {source}"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode}: rejected source emitted output"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).starts_with("orescranelift: "),
                "{mode}: expected controlled parser denial"
            );
        }
    }
}
