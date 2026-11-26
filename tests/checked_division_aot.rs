use std::{fs, process::Command};

#[cfg(unix)]
#[test]
fn checked_division_executes_native_objects() {
    let dir = std::env::temp_dir().join(format!("atto-div-aot-{}", std::process::id()));
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
  const int64_t values[] = {INT64_MIN, INT64_MIN+1, -100, -7, -1, 0, 1, 2, 7, 100, INT64_MAX};
  for (unsigned i=0;i<sizeof(values)/sizeof(values[0]);++i) {
    for (unsigned j=0;j<sizeof(values)/sizeof(values[0]);++j) {
      int64_t a=values[i], b=values[j], out=1234567;
      int32_t rc=oreslang_scalar_entry(a,b,0,0,&out);
      if (b==0) { assert(rc==2 && out==1234567); }
      else if (a==INT64_MIN && b==-1) { assert(rc==1 && out==1234567); }
      else { assert(rc==0 && out==a/b); }
    }
  }
}
"#).unwrap();
    fs::write(&source, "pub fnc main(a:int,b:int):int{return a/b;}").unwrap();
    let artifact = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("emit-object").arg(&source).output().unwrap();
    assert!(artifact.status.success(), "{}", String::from_utf8_lossy(&artifact.stderr));
    fs::write(&object, artifact.stdout).unwrap();
    let link=Command::new("cc").args(["-std=c11","-Wall","-Werror","-fsanitize=undefined","-fno-sanitize-recover=all"])
        .arg(&harness).arg(&object).arg("-o").arg(&binary).output().unwrap();
    assert!(link.status.success(), "{}",String::from_utf8_lossy(&link.stderr));
    assert!(Command::new(&binary).status().unwrap().success());
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(all(unix, target_pointer_width = "64"))]
#[test]
fn checked_division_native_object_fault_order_and_atomic_output() {
    // The existing checked-divisor gate covers the simple ABI. Extend it to
    // sequencing and discarded locals, with independent widened expectations.
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    let dir = std::env::temp_dir().join(format!("atto-div-order-aot-{}", std::process::id()));
    fs::create_dir(&dir).expect("exclusive test workspace");
    let _cleanup = Cleanup(dir.clone());
    let harness = dir.join("host.c");
    fs::write(&harness, include_str!("fixtures/checked_division_order.c")).unwrap();
    for (mode, guest) in [
        (0, "pub fnc main(a:int,b:int):int{return a/b;}"),
        (1, "pub fnc main(a:int,b:int):int{return (a+1)/b;}"),
        (2, "pub fnc main(a:int,b:int):int{return seq(a/b,9223372036854775807+1);}"),
        (3, "pub fnc main(a:int,b:int):int{val unused=a/b;return 42;}"),
    ] {
        let source=dir.join(format!("guest-{mode}.atto"));
        let object=dir.join(format!("guest-{mode}.o"));
        fs::write(&source, guest).unwrap();
        let artifact=Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object").arg(&source).output().unwrap();
        assert!(artifact.status.success(), "mode {mode}: {}", String::from_utf8_lossy(&artifact.stderr));
        assert!(artifact.stderr.is_empty(), "unexpected object-emission diagnostic");
        assert!(!artifact.stdout.is_empty(), "empty native object");
        fs::write(&object, artifact.stdout).unwrap();
        for opt in ["-O0", "-O2"] {
            let binary=dir.join(format!("guest-{mode}-{opt}"));
            let link=Command::new("cc").args(["-std=c11","-Wall","-Wextra","-Werror",
                "-fsanitize=undefined","-fno-sanitize-recover=all",opt])
                .arg(&harness).arg(&object).arg("-o").arg(&binary).output().unwrap();
            assert!(link.status.success(), "mode {mode}/{opt}: {}",String::from_utf8_lossy(&link.stderr));
            let run=Command::new(&binary).arg(mode.to_string()).output().unwrap();
            assert_eq!(run.status.code(),Some(0),"mode {mode}/{opt}: {}",String::from_utf8_lossy(&run.stderr));
            assert_eq!(run.stdout,b"CHECKED_DIVISION_AOT_ORDER_OK\n");
            assert!(run.stderr.is_empty());
        }
    }
}
