//! Linux object-AOT differential for closed scalar aggregate bindings.
//! The linker harness observes guest results; it is not a general noalloc proof.
#[cfg(target_os = "linux")]
#[test]
fn aggregate_objects_are_memory_independent_and_execute() {
    use std::{fs, process::Command};
    let dir = std::env::temp_dir().join(format!("atto-aggregate-aot-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(dir.clone());
    let src=dir.join("case.atto");
    let obj=dir.join("case.o");
    let c=dir.join("harness.c");
    let exe=dir.join("guest");
    fs::write(&c, r#"
#include <stdint.h>
#include <stdlib.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
static unsigned allocations, frees;
void *__real_malloc(size_t);
void __real_free(void *);
void *__wrap_malloc(size_t n) { ++allocations; return __real_malloc(n); }
void __wrap_free(void *p) { ++frees; __real_free(p); }
int main(int argc, char **argv) {
  assert(argc == 2);
  int64_t expected = strtoll(argv[1], 0, 10);
  for (unsigned i=0; i<100; ++i) {
    int64_t out = -1;
    assert(oreslang_scalar_entry(0,0,0,0,&out)==0 && out==expected);
  }
  assert(allocations==0 && frees==0);
  return 0;
}
"#).unwrap();
    for (body, expected) in [
        ("const Record r=(a=10);const Tuple t=(true,12);const struct s={a:20};return r.a+t[1]+s.a;", "42"),
        ("const Record<infer R> r=(a=-1);const Record<R> s=(a=43);return r.a+s.a;", "42"),
        ("const Record<infer R> r=(a=1,b=true);const Record<R> s=(b=false,a=11);const Tuple<infer T> t=(true,2);const Tuple<T> t2=(false,14);const struct<infer S> x={a:3};const struct<S> x2={a:23};return s.a+t2[1]+x2.a;", "48"),
    ] {
        fs::write(&src, format!("pub fnc main():int {{ {body} }}")).unwrap();
        let output=Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("emit-object").arg(&src).output().unwrap();
        assert!(output.status.success(), "{body}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stderr.is_empty());
        assert!(output.stdout.starts_with(b"\x7fELF"));
        fs::write(&obj, output.stdout).unwrap();
        let symbols=Command::new("nm").arg("-u").arg(&obj).output().unwrap();
        assert!(symbols.status.success());
        assert!(symbols.stdout.is_empty(), "unexpected external symbol: {}",String::from_utf8_lossy(&symbols.stdout));
        let link=Command::new("cc").args(["-std=c11","-Wall","-Wextra","-Werror","-Wl,--wrap=malloc","-Wl,--wrap=free"]).arg(&c).arg(&obj).arg("-o").arg(&exe).output().unwrap();
        assert!(link.status.success(), "{}",String::from_utf8_lossy(&link.stderr));
        assert!(Command::new(&exe).arg(expected).status().unwrap().success());
    }
    fs::write(&src,"pub fnc main():int { const Record<infer R> r=(a=1); const Record<R> s=(a=true); return 0; }").unwrap();
    let bad=Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("emit-object").arg(&src).output().unwrap();
    assert_eq!(bad.status.code(),Some(1));
    assert!(bad.stdout.is_empty());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("E_TYPE_MISMATCH"));
}
