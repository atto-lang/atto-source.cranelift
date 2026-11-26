//! Strict typed predicate Unicode NOT, real object execution, no JIT.
use std::{fs,process::Command};
#[cfg(target_os="linux")]
#[test]
fn unicode_not_native_predicate_and_no_artifact_denials(){
    let dir=std::env::temp_dir().join(format!("atto-unicode-not-{}",std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source=dir.join("guest.atto");
    let object=dir.join("guest.o");
    let driver=dir.join("driver.c");
    let binary=dir.join("guest");
    let compile=|body:&str|{
        fs::write(&source,format!("type R = int where self >= 1 and self <= 42; pub fnc main() -> int {{ {body} }}")).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("emit-object-strict").arg(&source).output().unwrap()
    };
    for body in [
        "if ¬false && ¬¬true { return 42; } else { return 0; }",
        "val x = 1; if ¬(x == 0) { return 42/x; } else { return 0; }",
    ] {
        let out=compile(body);
        assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
        fs::write(&object,out.stdout).unwrap();
        fs::write(&driver,"#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){int64_t out=-1;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==42);}\n").unwrap();
        assert!(Command::new("cc").arg(&driver).arg(&object).arg("-o").arg(&binary).status().unwrap().success());
        assert!(Command::new(&binary).status().unwrap().success());
    }
    for body in ["if ¬1 { return 42; } else { return 0; }","return ¬false;","if true¬ { return 42; } else { return 0; }"] {
        let out=compile(body);
        assert!(!out.status.success(),"{body}");
        assert!(out.stdout.is_empty(),"{body} emitted object");
        assert!(!out.stderr.is_empty());
    }
    fs::remove_dir_all(dir).unwrap();
}
