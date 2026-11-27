#![cfg(all(unix,target_pointer_width="64"))]
use std::{fs,process::Command};
const SOURCE:&str=include_str!("fixtures/attached_scalar_mutable.atto");
#[test]
fn native_attached_scalar_mutable_and_rejection() {
    let dir=std::env::temp_dir().join(format!("atto-attached-clif-{}",std::process::id()));
    fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    let _cleanup=Cleanup(dir.clone());
    let src=dir.join("guest.atto");let obj=dir.join("guest.o");let host=dir.join("host.c");let exe=dir.join("guest");
    let build=|source:&str,mode:&str|{
        fs::write(&src,source).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg(mode).arg(&src).output().unwrap()
    };
    fs::write(&host,r#"
#include <stdint.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void){int64_t out=17;int32_t status=oreslang_scalar_entry(0,0,0,0,&out);assert(status==STATUS);assert(out==EXPECTED);return 0;}
"#).unwrap();
    let overflow=SOURCE.replace("p.shift(10)","p.shift(9223372036854775807)");
    let flags="define struct Flag { bool value; } with { constructor(bool value) { self.value = value; } }\n";
    let lazy=format!("{flags}{}", SOURCE.replace("p.shift(10);","p.shift(10); let a = Flag(false && (9223372036854775807 + 1 == 0)); let b = Flag(true || (9223372036854775807 + 1 == 0)); let c = Flag(p.is_origin());"));
    let eager=lazy.replace("false && (9223372036854775807","true && (9223372036854775807");
    let ordered=SOURCE.replace("self.b.y = self.b.y + n;","self.b.y = self.b.y + self.a.x;");
    let snapshot=SOURCE.replace("r.shift(1)","r.shift(r.a.x)");
    for (source,status,expected) in [(SOURCE,0,1316),(&overflow,1,17),(&lazy,0,1316),(&eager,1,17),(&ordered,0,1336),(&snapshot,0,1316)]{
        let a=build(source,"emit-attached-struct-object");assert!(a.status.success(),"{:?}",a);
        assert!(!a.stdout.is_empty());
        let b=build(source,"emit-attached-struct-object");assert_eq!(a.stdout,b.stdout);
        fs::write(&obj,a.stdout).unwrap();
        for opt in ["-O0","-O2"]{
            let link=Command::new("cc").args(["-std=c11","-Wall","-Wextra","-Werror",opt])
                .arg(format!("-DSTATUS={status}")).arg(format!("-DEXPECTED={expected}")).arg(&host).arg(&obj).arg("-o").arg(&exe).output().unwrap();
            assert!(link.status.success(),"{:?}",link);assert!(Command::new(&exe).status().unwrap().success());
            println!("ATTACHED_SCALAR_NATIVE_PASS expected_status={status} expected_result={expected} optimization={opt}");
        }
    }
    for invalid in [
        SOURCE.replace("let mut Rectangle","let Rectangle"),
        SOURCE.replace("r.shift(1)","r.shift(true)"),
        SOURCE.replace("self.a.x = self.a.x + n","self.a.z = self.a.x + n"),
        SOURCE.replace("self.b = b;",""),
        SOURCE.replace("Point a;","Rectangle a;"),
        SOURCE.replace("let mut Rectangle","let mut Point"),
        SOURCE.replace("Rectangle(p,q)","Rectangle(p,42)"),
        SOURCE.replace("copy! r","copy!! r"),
        SOURCE.replace("copy! r","copy ! r"),
        SOURCE.replace("p.shift(10);","let invalid = p.shift(10);"),
        format!("{SOURCE} fnc extra():int{{return 0;}}"),
    ]{
        let a=build(&invalid,"emit-attached-struct-object");
        assert!(!a.status.success()&&a.stdout.is_empty(),"{invalid}: {:?}",a);
    }
    let ordinary=build(SOURCE,"emit-object");assert!(!ordinary.status.success()&&ordinary.stdout.is_empty());
}
