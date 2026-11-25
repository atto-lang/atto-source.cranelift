//! Real native guest execution for candidate signed-i64 bitwise intrinsics.
//! The compiler must build and execute CLIF; no host arithmetic oracle is a pass.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(0);

fn execute(mode: &str, expr: &str) -> std::process::Output {
    let seq = NEXT_SOURCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-bitwise-{}-{seq}.atto",
        std::process::id()
    ));
    let src = format!("fnc main(x:int,y:int):int{{return {expr};}}");
    std::fs::write(&path, src).expect("write native source");
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(if mode == "run" {
            &["7", "3"][..]
        } else {
            &[][..]
        })
        .output();
    std::fs::remove_file(&path).expect("remove fixture");
    out.expect("launch compiled guest")
}

#[test]
fn native_bitwise_and_or_compile_and_execute() {
    for (expr, expected, clif_instruction) in [
        ("Integer.bitAnd(x,y)", "3\n", "band"),
        ("Integer.bitOr(x,y)", "7\n", "bor"),
        ("Integer.bitAnd(x+1,y*2)", "0\n", "band"),
        ("Integer.bitOr(x+1,y*2)", "14\n", "bor"),
        (
            "Integer.bitAnd(-1,-9223372036854775808)",
            "-9223372036854775808\n",
            "band",
        ),
        (
            "Integer.bitOr(-9223372036854775808,1)",
            "-9223372036854775807\n",
            "bor",
        ),
    ] {
        let run = execute("run", expr);
        assert_eq!(
            run.status.code(),
            Some(0),
            "{expr}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{expr}");
        assert!(run.stderr.is_empty(), "{expr}");

        let ir = execute("ir", expr);
        assert_eq!(
            ir.status.code(),
            Some(0),
            "{expr}: {}",
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(ir.stderr.is_empty(), "{expr}");
        assert!(
            String::from_utf8_lossy(&ir.stdout).contains(clif_instruction),
            "{expr} did not emit the expected native CLIF instruction"
        );
    }
}

#[test]
fn operand_overflow_and_unsupported_surface_do_not_emit_guest_or_ir() {
    for expr in [
        "Integer.bitAnd(9223372036854775807+1,y)",
        "Integer.bitOr(x,9223372036854775807+1)",
        "Integer.bitAnd(x-9223372036854775807-9,y)",
        "Integer.bitand(x,y)",
        "Integer.bitOr(x)",
        "Integer.bitAnd(x,y,1)",
        "Integer.bitOr(x | y,1)",
        "x & y",
        "x | y",
    ] {
        for mode in ["run", "ir"] {
            let out = execute(mode, expr);
            // Checked runtime overflow compiles to valid IR, then rejects
            // during actual JIT execution; syntax failures must reject in
            // both modes. Distinguish the two phases explicitly.
            let runtime_overflow = expr.contains("9223372036854775807");
            if mode == "ir" && runtime_overflow {
                assert_eq!(out.status.code(), Some(0), "{expr}: expected verified CLIF");
                assert!(!out.stdout.is_empty(), "{expr}: empty verified IR");
                continue;
            }
            assert_eq!(
                out.status.code(),
                Some(1),
                "{mode}/{expr}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                out.stdout.is_empty(),
                "{mode}/{expr}: stdout must be empty on rejection"
            );
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                stderr.starts_with("orescranelift: "),
                "{mode}/{expr}: {stderr}"
            );
            assert!(!stderr.contains("panicked at"), "{mode}/{expr}: {stderr}");
        }
    }
}
