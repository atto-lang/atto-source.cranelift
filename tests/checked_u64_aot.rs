//! Full-width native ABI observer. Not a typed guest Error or helper-call proof.
#![cfg(all(unix, target_pointer_width = "64"))]
use std::{fs, process::Command};

#[test]
fn unsigned_native_object_matrix() {
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let dir = std::env::temp_dir().join(format!("atto-u64-clif-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let _cleanup = Cleanup(dir.clone());
    let src = dir.join("guest.atto");
    let obj = dir.join("guest.o");
    let host = dir.join("host.c");
    let bin = dir.join("guest");
    fs::write(&host, r#"
#include <stdint.h>
#include <assert.h>
extern int32_t atto_u64_scalar_entry(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t*);
int main(void) {
    const uint64_t v[] = {0,1,2,UINT64_C(4294967296),UINT64_C(9007199254740993),UINT64_C(9223372036854775808),UINT64_MAX-1,UINT64_MAX};
    for (unsigned i=0;i<sizeof(v)/sizeof(v[0]);++i) for (unsigned j=0;j<sizeof(v)/sizeof(v[0]);++j) {
        uint64_t a=v[i], b=v[j], out=12345, expect=0; int status=0;
#if OP == 0
        if (b>UINT64_MAX-a) status=1; else expect=a+b;
#elif OP == 1
        if (b>a) status=1; else expect=a-b;
#elif OP == 2
        if (b && a>UINT64_MAX/b) status=1; else expect=a*b;
#else
        if (!b) status=2; else expect=a/b;
#endif
        assert(atto_u64_scalar_entry(a,b,0,0,&out)==status);
        assert(out==(status?12345:expect));
    }
    return 0;
}
"#).unwrap();
    for (op, spelling) in ["+", "-", "*", "/"].into_iter().enumerate() {
        fs::write(
            &src,
            format!("pub fnc main(u64 a,b:uint):u64{{return a{spelling}b;}}"),
        )
        .unwrap();
        let artifact = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object-u64")
            .arg(&src)
            .output()
            .unwrap();
        assert!(artifact.status.success(), "{:?}", artifact);
        assert!(!artifact.stdout.is_empty());
        fs::write(&obj, artifact.stdout).unwrap();
        for opt in ["-O0", "-O2"] {
            let link = Command::new("cc")
                .args(["-std=c11", "-Wall", "-Wextra", "-Werror", opt])
                .arg(format!("-DOP={op}"))
                .arg(&host)
                .arg(&obj)
                .arg("-o")
                .arg(&bin)
                .output()
                .unwrap();
            assert!(link.status.success(), "{:?}", link);
            assert!(Command::new(&bin).status().unwrap().success());
        }
    }
    // Exact bit pattern and intermediate overflow even when the result cancels.
    fs::write(&host, "#include <stdint.h>\n#include <assert.h>\nextern int32_t atto_u64_scalar_entry(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t*);\nint main(void){uint64_t out=17;int rc=atto_u64_scalar_entry(0,0,0,0,&out);assert(rc==STATUS);assert(out==EXPECTED);return 0;}\n").unwrap();
    for (expr, status, expected) in [
        ("18446744073709551615", 0, "UINT64_MAX"),
        ("(18446744073709551615+1)*0", 1, "17"),
        ("(0-1)+1", 1, "17"),
        ("1/0", 2, "17"),
    ] {
        fs::write(&src, format!("pub fnc main():u64{{return {expr};}}")).unwrap();
        let artifact = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object-u64")
            .arg(&src)
            .output()
            .unwrap();
        assert!(artifact.status.success(), "{:?}", artifact);
        fs::write(&obj, artifact.stdout).unwrap();
        let link = Command::new("cc")
            .arg(format!("-DSTATUS={status}"))
            .arg(format!("-DEXPECTED={expected}"))
            .arg(&host)
            .arg(&obj)
            .arg("-o")
            .arg(&bin)
            .output()
            .unwrap();
        assert!(link.status.success(), "{:?}", link);
        assert!(Command::new(&bin).status().unwrap().success());
    }
    for invalid in [
        "pub fnc main():int{return 1;}",
        "pub fnc main():u64{return 18446744073709551616;}",
        "pub fnc main():u64{return -1;}",
        "pub fnc main():u64{return 1 as int;}",
        "pub fnc main():u64{return 1 >> 8;}",
        "pub fnc main(int a):u64{return a;}",
        "pub fnc main():u64{return f();}",
        "pub fnc main():u64{return 1;} fnc f():u64{return 1;}",
    ] {
        fs::write(&src, invalid).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object-u64")
            .arg(&src)
            .output()
            .unwrap();
        assert!(
            !result.status.success() && result.stdout.is_empty(),
            "{invalid}"
        );
    }
    fs::write(&src, "pub fnc main():u64{return 1;}").unwrap();
    let signed = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("emit-object")
        .arg(&src)
        .output()
        .unwrap();
    assert!(!signed.status.success() && signed.stdout.is_empty());
}
