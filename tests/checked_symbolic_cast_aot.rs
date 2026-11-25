use std::fs;
use std::process::Command;

#[cfg(unix)]
#[test]
fn checked_cast_aliases_execute_linked_native_objects() {
    let dir = std::env::temp_dir().join(format!("atto-cast-aot-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("guest.atto");
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    let binary = dir.join("guest");
    fs::write(&harness, r#"
#include <stdint.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void) {
  int64_t out=0;
  assert(oreslang_scalar_entry(41,0,0,0,&out)==0 && out==42);
  assert(oreslang_scalar_entry(INT64_MAX,0,0,0,&out)==1 && out==42);
}
"#).unwrap();
    for expr in ["(x + 1) as i64", "<i64>(x + 1)", "(x + 1)::<i64>",
                 "x::<int> + 1", "<int>x + 1"] {
        fs::write(&source, format!("pub fnc main(x:int):int{{return {expr};}}")).unwrap();
        let artifact = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object").arg(&source).output().unwrap();
        assert!(artifact.status.success(), "{expr}: {}", String::from_utf8_lossy(&artifact.stderr));
        fs::write(&object, artifact.stdout).unwrap();
        let link = Command::new("cc").args(["-std=c11","-Wall","-Werror"])
            .arg(&harness).arg(&object).arg("-o").arg(&binary).output().unwrap();
        assert!(link.status.success(), "{}",String::from_utf8_lossy(&link.stderr));
        assert!(Command::new(&binary).status().unwrap().success(), "{expr}");
    }
    for expr in ["<bool>42","42::<bool>","42 as bool","42::<i32>","<Second>42",
                 "42.<Second>","42::<i64","<i64 42","42 as? i64"] {
        fs::write(&source, format!("pub fnc main():int{{return {expr};}}")).unwrap();
        let out=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object").arg(&source).output().unwrap();
        assert_eq!(out.status.code(),Some(1),"{expr}");
        assert!(out.stdout.is_empty(),"{expr} emitted rejected object");
    }
    fs::remove_dir_all(&dir).unwrap();
}
