#[cfg(target_os = "linux")]
#[test]
fn named_recursive_native_guests_and_denials() {
    use std::{fs, process::Command};
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) { let _=fs::remove_dir_all(&self.0); }
    }
    let dir=std::env::temp_dir().join(format!("atto-named-recursive-{}",std::process::id()));
    fs::create_dir(&dir).expect("exclusive test workspace");
    let _cleanup=Cleanup(dir.clone());
    let compiler=env!("CARGO_BIN_EXE_orescranelift");
    let run=|mode:&str,src:&str|{
        let path=dir.join("main.atto");
        fs::write(&path,src).unwrap();
        Command::new("timeout").args(["30s",compiler,mode]).arg(path).output().unwrap()
    };
    let fib=|n:i64|format!("pub fnc main(): int {{return {n} |> fnc fib(int v): int {{^ match v over on 0 -> 0, on 1 -> 1, on _ -> fib(v-2)+fib(v-1) end }};}}");
    let count=|n:i64|format!("pub fnc main(): int {{return {n} |> fnc count(int v): int {{^ match v over on 0 -> 0, on _ -> count(v-1)+1 end;}};}}");
    for (source,want) in [(fib(0),"0\n"),(fib(1),"1\n"),(fib(10),"55\n"),(fib(20),"6765\n"),(count(600),"600\n"),(count(1024),"1024\n")]{
        let checked=run("check",&source);
        assert!(checked.status.success(),"{:?}",checked);
        assert!(checked.stdout.is_empty()&&checked.stderr.is_empty());
        let guest=run("run",&source);
        assert_eq!(guest.status.code(),Some(0),"{:?}",guest);
        assert_eq!(guest.stdout,want.as_bytes());
        assert!(guest.stderr.is_empty());
    }
    for (source,diagnostic) in [
        (count(1025),"orescranelift: E_STACK\n"),
        ("pub fnc main(): int {return 9223372036854775807 |> fnc f(int v): int {return v+1;};}".into(),"orescranelift: i64 arithmetic overflow\n"),
        ("pub fnc main(): int {return 0 |> fnc f(int v): int {return f(v);};}".into(),"orescranelift: E_STACK\n"),
    ] {
        let guest=run("run",&source);
        assert_eq!(guest.status.code(),Some(1),"{:?}",guest);
        assert!(guest.stdout.is_empty());
        assert_eq!(guest.stderr,diagnostic.as_bytes());
    }
    let exact="pub fnc main(): int {\n    return 55 |> fnc fib(int v): int {\n        ^ match v over\n            on 0 -> 0,\n            on 1 -> 1,\n            on _ -> fib(v - 2) + fib(v - 1)\n        end\n    };\n}\n";
    let ir=run("ir",exact);
    assert!(ir.status.success(),"{:?}",ir);
    assert!(ir.stderr.is_empty());
    assert!(String::from_utf8(ir.stdout).unwrap().contains("call "));
    // Exponential fixture deliberately not run. IR alone is not guest proof.
    for (source,code) in [
        ("pub fnc main(): int {return 1 |> fnc f(int v): int {return v+x;};}","E_NAMED_STAGE_CLOSED_INT"),
        ("pub fnc main(): int {return 1 |> fnc f(int v): int {return f(v,v);};}","E_NAMED_STAGE_CLOSED_INT"),
        ("pub fnc main(): int {return 1 |> fnc f(int f): int {return f;};}","E_NAMED_STAGE_NAME"),
        ("pub fnc main(): int {return 1 |> fnc f(int v): int {^ match v over on 0 -> 0 end};}","E_NAMED_STAGE_NONEXHAUSTIVE"),
        ("pub fnc main(): int {return 1 |> fnc f(int v): int {^ match v over on 0 -> 0, on 0 -> 1, on _ -> 2 end};}","E_NAMED_STAGE_DUP_PATTERN"),
        ("pub fnc main(): int {return 1 |> fnc f(int v): int {return f(v);};} fnc x(): int {return 42;}","E_NAMED_STAGE_TRAILING"),
        ("pub fnc main(): int {return 1+2 |> fnc f(int v): int {return v;};}","E_NAMED_STAGE_LITERAL_UPSTREAM"),
    ] {
        for mode in ["check","run","ir"]{
            let denied=run(mode,source);
            assert_eq!(denied.status.code(),Some(1),"{:?}",denied);
            assert!(denied.stdout.is_empty());
            assert_eq!(String::from_utf8(denied.stderr).unwrap(),format!("orescranelift: {code}\n"));
        }
    }
    for mode in ["emit-object","isolate","run-traced"] {
        let denied=run(mode,&fib(10));
        assert_eq!(denied.status.code(),Some(1),"{:?}",denied);
        assert!(denied.stdout.is_empty());
    }
    // Existing ordinary recursion must remain forbidden, even if dormant.
    let ordinary=run("check","pub fnc main(): int {return 42;} fnc f(int v): int {return f(v);}");
    assert_eq!(ordinary.status.code(),Some(1));
    assert!(ordinary.stdout.is_empty());
    assert_eq!(ordinary.stderr,b"orescranelift: E_LINK_CALL_CYCLE\n");
}
