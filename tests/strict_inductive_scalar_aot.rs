use std::{fs, process::Command};
#[cfg(unix)]
#[test]
fn strict_inductive_scalar_native_guests() {
    let dir=std::env::temp_dir().join(format!("atto-strict-inductive-{}",std::process::id()));
    fs::create_dir(&dir).unwrap();
    let pre="define dimension Time; define unit Second: Time; type Bounded = int where self >= 1 and self <= 42;";
    let source=dir.join("guest.atto");let object=dir.join("guest.o");let harness=dir.join("host.c");let binary=dir.join("guest");
    let compile=|body:&str| {
        fs::write(&source,format!("{pre} {body}")).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("emit-object-strict").arg(&source).output().unwrap()
    };
    for body in [
        "pub fnc main() -> int { let int x=1; x=42; return x; }",
        "pub fnc main() -> int { let Bounded x=1; while x < 42 { x=x+1; } return x; }",
        "pub fnc main() -> int { let int x=42; while x > 0 { x=x-1; } return x+42; }",
        "pub fnc main() -> int { let int x=0; while x < 21 { x=x+1; } while x < 42 { x=x+1; } return x; }",
        "pub fnc main() -> int { let int x=41; val old=x; x=1; return old+x; }",
        "fnc f(Bounded n) -> int { let int x=n; while x < 42 { x=x+1; } return x; } pub fnc main() -> int { return f(1); }",
        "pub fnc main() -> int { let int x=42; while x < 0 && 42/x > 0 { x=x+1; } return x; }",
    ] {
        let out=compile(body);
        assert!(out.status.success(),"{body}: {}",String::from_utf8_lossy(&out.stderr));
        assert!(!out.stdout.is_empty());
        fs::write(&object,out.stdout).unwrap();
        fs::write(&harness,"#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){int64_t out=123;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==42);}").unwrap();
        let link=Command::new("cc").args(["-std=c11","-Wall","-Werror"]).arg(&harness).arg(&object).arg("-o").arg(&binary).output().unwrap();
        assert!(link.status.success(),"{}",String::from_utf8_lossy(&link.stderr));
        assert!(Command::new("timeout").arg("10").arg(&binary).status().unwrap().success(),"{body}");
        println!("STRICT_LOOP_NATIVE_PASS {body}");
    }
    for (body,code) in [
        ("pub fnc main() -> int { let int x=1; x=0; return 42/x; }","E_VERIFY_UNPROVEN"),
        ("pub fnc main() -> int { let int x=0; while x >= 0 { x=x+1; } return x; }","E_VERIFY_UNPROVEN"),
        ("pub fnc main() -> int { let Bounded x=1; while x <= 42 { x=x+1; } return x; }","E_REFINEMENT_BINDING"),
        ("pub fnc main() -> int { let int x=1; while x > 0 { x=x-1; } return 42/x; }","E_VERIFY_UNPROVEN"),
        ("pub fnc main() -> int { val x=1; x=0; return x; }","E_VERIFY_UNSUPPORTED"),
        ("pub fnc main() -> int { let int x=0; while x < 42 { x=x+1; x=x+1; } return x; }","E_VERIFY_UNSUPPORTED"),
    ] {
        let out=compile(body);
        assert!(!out.status.success(),"{body}");
        assert!(out.stdout.is_empty(),"{body} emitted object");
        assert!(String::from_utf8_lossy(&out.stderr).contains(code),"{body}: {}",String::from_utf8_lossy(&out.stderr));
        println!("STRICT_LOOP_NATIVE_REJECT {body}");
    }
    fs::remove_dir_all(dir).unwrap();
}
