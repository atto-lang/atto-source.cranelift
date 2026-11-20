//! Independent native Cranelift admission evidence for canonical Atto local
//! bindings. Source PR #34 was a legacy proposal, not the current atto-spec.
use std::io::Write;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn guest(mode: &str, source: &str, args: &[&str]) -> Output {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-local-{}-{n}.atto", std::process::id()
    ));
    // Refuse pre-existing paths and symlink leaves; never truncate someone
    // else's source fixture in a shared temporary directory.
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .expect("create exclusive local-binding fixture");
    file.write_all(source.as_bytes()).expect("write source fixture");
    drop(file);
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .args(args)
        .output()
        .expect("launch real Cranelift executable");
    std::fs::remove_file(&path).expect("remove source fixture");
    result
}

#[test]
fn local_const_and_val_have_distinct_native_semantics() {
    for (source, args, answer) in [
        ("fnc main():int{const x=40+2;return x;}", &[][..], b"42\n".as_slice()),
        ("fnc main():int{int x=40+2;return x;}", &[][..], b"42\n".as_slice()),
        ("fnc main():int{const int x=40;const y=x+2;return y;}", &[][..], b"42\n".as_slice()),
        ("fnc main(a:int):int{val y=a+1;return y;}", &["41"][..], b"42\n".as_slice()),
        ("fnc main(a:int):int{val int y=a+1;val z=y+1;return z;}", &["40"][..], b"42\n".as_slice()),
        ("fnc main(a:int):int{val y=a+1;const z=40+2;return z;}", &["0"][..], b"42\n".as_slice()),
    ] {
        let output = guest("run", source, args);
        assert_eq!(output.status.code(), Some(0), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, answer, "{source}");
        assert!(output.stderr.is_empty(), "{source}");
        let ir = guest("ir", source, &[]);
        assert_eq!(ir.status.code(), Some(0), "{source}: {}", String::from_utf8_lossy(&ir.stderr));
        assert!(!ir.stdout.is_empty(), "{source}: native CLIF required");
    }
}

#[test]
fn runtime_const_and_type_first_parameter_must_fail_before_ir() {
    for source in [
        "fnc main(a:int):int{const x=a+1;return x;}",
        "fnc main(a:int):int{int x=a+1;return x;}",
        "fnc main(a:int):int{val y=a+1;const x=y+1;return x;}",
        "fnc main():int{const x=9223372036854775807+1;return 0;}",
        "fnc main():int{int x=9223372036854775807+1;return 0;}",
        "fnc main():int{const x=Integer.pow(2,3);return x;}",
        "fnc main():int{val y=y+1;return y;}",
        "fnc main():int{const x=x+1;return x;}",
        "fnc main(a:int):int{val a=2;return a;}",
        "fnc main():int{const x=3;val x=4;return x;}",
        "fnc main():int{let x=1;return x;}",
        "fnc main():int{const x=1;x=2;return x;}",
    ] {
        for mode in ["check", "ir", "run"] {
            let args = if source.contains("main(a:int)") && mode == "run" { &["41"][..] } else { &[][..] };
            let out = guest(mode, source, args);
            assert_eq!(out.status.code(), Some(1), "{mode}: {source}");
            assert!(out.stdout.is_empty(), "{mode}: no CLIF/guest output on rejection");
            let diag = String::from_utf8_lossy(&out.stderr);
            assert!(diag.starts_with("orescranelift: "), "{source}: {diag}");
            assert!(!diag.contains("panicked at"), "{source}: {diag}");
        }
    }
}

#[test]
fn discarded_val_overflow_is_runtime_failure_not_compile_time_failure() {
    let source = "fnc main(a:int):int{val unused=a+1;return 42;}";
    let ir = guest("ir", source, &[]);
    assert_eq!(ir.status.code(), Some(0), "runtime overflow must have valid CLIF");
    assert!(!ir.stdout.is_empty());
    let safe = guest("run", source, &["0"]);
    assert_eq!(safe.status.code(), Some(0));
    assert_eq!(safe.stdout, b"42\n");
    let overflow = guest("run", source, &["9223372036854775807"]);
    assert_eq!(overflow.status.code(), Some(1));
    assert!(overflow.stdout.is_empty());
    assert!(String::from_utf8_lossy(&overflow.stderr).contains("overflow"));
}

#[test]
fn binding_budget_and_comment_inertness() {
    let mut ok = String::from("fnc main():int{");
    for index in 0..64 {
        ok.push_str(&format!("const v{index}={index};"));
    }
    ok.push_str("return v63;}");
    assert_eq!(guest("run", &ok, &[]).stdout, b"63\n");
    let invalid = ok.replace("return v63;", "const v64=64;return v63;");
    for mode in ["run", "ir"] {
        let result = guest(mode, &invalid, &[]);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
    }
    let comments = "fnc main():int{/* const x=a+1 */ val x=40; // let y=bad\nreturn x+2;}";
    assert_eq!(guest("run", comments, &[]).stdout, b"42\n");
}
