use std::fs;
use std::io::Write;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn run(source: &str, mode: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "atto-predicates-{}-{}.atto", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    file.write_all(source.as_bytes()).unwrap();
    drop(file);
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
    let _cleanup = Cleanup(path.clone());
    Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg(mode).arg(path).output().unwrap()
}
#[test]
fn linked_native_arena_predicates_execute() {
    let helper = "pub fnc alignment(int a):bool { if a <= 0 then return false; fi return (a & (a - 1)) == 0; } ";
    for (expr, expected) in [
        ("if alignment(8) then return 42; fi return 0;", "42"),
        ("if alignment(3) then return 0; fi return 42;", "42"),
        ("if alignment(0 - 1) then return 0; fi return 42;", "42"),
        ("if (0 - 1) < 0 && 0 <= 0 && 7 > 3 && 7 >= 7 then return 42; fi return 0;", "42"),
        ("return (0 - 1) & 42;", "42"),
        ("return 1 + 3 & 6;", "4"),
    ] {
        let source=format!("{helper} pub fnc main():int {{ {expr} }}");
        let out=run(&source,"run");
        assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(),expected);
        let ir=run(&source,"ir");
        assert!(ir.status.success());
        assert!(String::from_utf8_lossy(&ir.stdout).contains("band"));
    }
    for expr in ["return true & false;", "return 12 & true;", "return 12 & 10 == 8;", "return true < false;", "return 12 & & 10;"] {
        let source=format!("{helper} pub fnc main():int {{ {expr} }}");
        for mode in ["check","run","ir"] {
            let out=run(&source,mode);
            assert!(!out.status.success()&&out.stdout.is_empty(),"{mode}: {expr}");
        }
    }
}
