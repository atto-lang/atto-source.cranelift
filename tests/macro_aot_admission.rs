//! Real compiler CLI admission tests, not a macro-expansion implementation.
//! Cranelift stage-0 must never reinterpret compiler-only macros as runtime
//! scalar calls, emit CLIF, or fall back to a JIT when an AOT macro is needed.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const DIAGNOSTIC: &str = "CRANELIFT_MACRO_EXPANDER_REQUIRED";

fn compile(source: &str, mode: &str, ext: &str) -> Output {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-macro-admission-{}-{sequence}.{ext}",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write macro admission fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("invoke actual Cranelift frontend");
    std::fs::remove_file(&path).expect("remove fixture");
    output
}

#[test]
fn declared_and_invoked_macros_fail_closed_before_codegen() {
    let bad_sources = [
        // The spelling and modifiers must be recognized even when unused.
        "define macro identity(Syntax<Expr> value) -> Syntax<Expr> { return value; }\nfnc main():int{return 42;}",
        "define pub macro literal(Const<int> value) -> Syntax<Expr> { return quote(value); }\nfnc main():int{return 42;}",
        "pub define macro identity(Syntax<Expr> value) -> Syntax<Expr> { return value; }\nfnc main():int{return 42;}",
        "define // one-line comment\n macro identity(Syntax<Expr> value) -> Syntax<Expr> { return value; }",
        "fnc main():int{return 42;}\ndefine macro unused(Syntax<Expr> value) -> Syntax<Expr> { return value; }",
        // Invocations are never runtime calls or tolerated unknown symbols.
        "fnc main():int{return identity!(syntax expr 42);}",
        "fnc main():int{return literal!(const 40 + 2);}",
        "fnc main():int{return unknown!(const 42);}",
        "fnc main():int{return quote!(const 42);}",
        "fnc main():int{return 42!(const 42);}",
        "fnc main():int{return identity ! (syntax expr 42);}",
        "fnc main():int{return 42;}\nunknown!(const 42)",
    ];
    for source in bad_sources {
        for ext in ["atto", "ores"] {
            for mode in ["run", "ir", "isolate"] {
                let output = compile(source, mode, ext);
                assert!(
                    !output.status.success(),
                    "{mode}/{ext} admitted unsupported macro source: {source}"
                );
                assert!(
                    output.stdout.is_empty(),
                    "{mode}/{ext} emitted guest output/IR for unsupported macro: {source}"
                );
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    stderr.contains(DIAGNOSTIC),
                    "{mode}/{ext} failed with unrelated error for {source}: {stderr}"
                );
            }
        }
    }
}

#[test]
fn comments_do_not_mint_macros_or_block_ordinary_scalar_aot_inputs() {
    let ordinary_sources = [
        "fnc main():int{return 42;} // define macro rejected!",
        "// literal!(const 40 + 2)\nfnc main():int{return 42;}",
        "// define macro fake\rfnc main():int{return 42;}",
        "define fnc main():int{return (20+1)*2;} // identity!(syntax expr 0)",
    ];
    for source in ordinary_sources {
        for ext in ["atto", "ores"] {
            let output = compile(source, "run", ext);
            assert!(
                output.status.success(),
                "valid scalar source rejected for {ext}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, b"42\n");
            let ir = compile(source, "ir", ext);
            assert!(
                ir.status.success(),
                "IR rejected: {}",
                String::from_utf8_lossy(&ir.stderr)
            );
            assert!(
                !ir.stdout.is_empty(),
                "ordinary scalar CLIF must be emitted"
            );
        }
    }
}
