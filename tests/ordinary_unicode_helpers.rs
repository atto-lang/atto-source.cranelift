//! Ordinary native helper dispatch: typed Unicode names, eager checked arguments,
//! and rejection before emitting object bytes.
#[cfg(target_os = "linux")]
#[test]
fn ordinary_unicode_helpers_link_and_execute() {
    use std::{fs,process::Command};
    let dir = std::env::temp_dir().join(format!("atto-plain-helper-{}",std::process::id()));
    fs::create_dir(&dir).unwrap();
    let src = dir.join("guest.atto");
    let obj = dir.join("guest.o");
    let driver = dir.join("driver.c");
    let bin = dir.join("guest");
    let compile = |source: &str| {
        fs::write(&src, source).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("emit-object").arg(&src).output().unwrap()
    };
    let out = compile("fnc 合計(int Φ, int φ, int 数量, int 𠮷): int { return Φ + φ + 数量 + 𠮷; } pub fnc main(): int { return 合計(10,20,30,40); }");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    fs::write(&obj, out.stdout).unwrap();
    fs::write(&driver, "#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){int64_t out=-1;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==100);}").unwrap();
    assert!(Command::new("cc").arg(&driver).arg(&obj).arg("-o").arg(&bin).status().unwrap().success());
    assert!(Command::new(&bin).status().unwrap().success());
    for source in [
        "fnc 合計(int Φ,int Φ):int {return Φ;} pub fnc main():int {return 合計(1,2);}",
        "fnc 合計(int Φ):int {return 未定義;} pub fnc main():int {return 合計(1);}",
        "fnc 合計(int Φ):int {return Φ;} pub fnc main():int {return 合計();}",
        "fnc 合計(int Φ):int {return Φ;} pub fnc main():int {return 合計(1,2);}",
        "fnc 合計(int Φ):int {return 合計(Φ);} pub fnc main():int {return 合計(1);}",
        "fnc 合計(int Φ):int {return Φ;} pub fnc main():int {return 合計(true);}",
    ] {
        let out=compile(source);
        assert_eq!(out.status.code(),Some(1),"{source}");
        assert!(out.stdout.is_empty(),"{source} emitted object");
        assert!(!out.stderr.is_empty());
    }
    fs::remove_dir_all(dir).unwrap();
}
