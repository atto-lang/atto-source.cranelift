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
    fs::write(
        &harness,
        r#"
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
"#,
    )
    .unwrap();
    fs::write(&source, "pub fnc main(a:int,b:int):int{return a/b;}").unwrap();
    let artifact = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("emit-object")
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        artifact.status.success(),
        "{}",
        String::from_utf8_lossy(&artifact.stderr)
    );
    fs::write(&object, artifact.stdout).unwrap();
    let link = Command::new("cc")
        .args([
            "-std=c11",
            "-Wall",
            "-Werror",
            "-fsanitize=undefined",
            "-fno-sanitize-recover=all",
        ])
        .arg(&harness)
        .arg(&object)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        link.status.success(),
        "{}",
        String::from_utf8_lossy(&link.stderr)
    );
    assert!(Command::new(&binary).status().unwrap().success());
    fs::remove_dir_all(&dir).unwrap();
}
