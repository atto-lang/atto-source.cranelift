use std::{fs, process::Command};
#[cfg(unix)]
#[test]
fn immutable_quantity_bindings_execute_once_in_order() {
    let dir = std::env::temp_dir().join(format!("atto-bindings-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    let source = dir.join("guest.atto");
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    let binary = dir.join("guest");
    let pre = "define dimension Time; define dimension Length; define unit Second: Time; define unit Meter: Length; define unit Millis: Time = Second / 1000; type Positive = int where self > 0 and units in Second; type Bounded = int where self >= 1 and self <= 42;";
    let compile = |body: &str| {
        fs::write(&source, format!("{pre} pub fnc main() -> int {{{body}}}")).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object")
            .arg(&source)
            .output()
            .unwrap()
    };
    for (body, status, value) in [
        ("val a = 42<Second>; return a/1<Second>;", 0, 42),
        ("val Positive a = 42<Second>; return a/1<Second>;", 0, 42),
        ("val a: Positive = 42<Second>; return a/1<Second>;", 0, 42),
        (
            "val a = 42000<Millis>.to_unit<Second>(); val Positive b=a; return b/1<Second>;",
            0,
            42,
        ),
        ("val a = 21<Second>; return (a+a)/1<Second>;", 0, 42),
        ("val Bounded a = 40+2; return a;", 0, 42),
        ("val a = 43::<Bounded>; return 42;", 7, 123),
        ("val unused = 1<Second>/0<Second>; return 42;", 2, 123),
        (
            "val first = 43::<Bounded>; val second=9223372036854775807+1; return 42;",
            7,
            123,
        ),
        (
            "val first=9223372036854775807+1; val second=43::<Bounded>; return 42;",
            1,
            123,
        ),
        ("val a=42; val b=a.<Second>; return b/1<Second>+a-a;", 0, 42),
        ("val a=1::<Bounded>; val Bounded b=a; return b+41;", 0, 42),
    ] {
        let out = compile(body);
        assert!(
            out.status.success(),
            "{body}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::write(&object, out.stdout).unwrap();
        fs::write(&harness,format!("#include <stdint.h>\n#include <assert.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){{int64_t out=123;assert(oreslang_scalar_entry(0,0,0,0,&out)=={status});assert(out=={value});}}")).unwrap();
        let link = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Werror"])
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
        assert!(Command::new(&binary).status().unwrap().success(), "{body}");
        println!("BINDING_NATIVE_PASS {body}");
    }
    for body in [
        "val a=42<Second>; return a;",
        "val a=42<Second>; val int b=a; return b;",
        "val Positive a=0<Second>; return 42;",
        "val Positive a=42<Millis>; return 42;",
        "val Bounded a=43; return a;",
        "val a=b; val b=42; return a;",
        "val a=a; return a;",
        "val a=1; val a=42; return a;",
        "val a=42; a=0; return a;",
        "val a=42; return a.<Second>;",
        "val self=42; return self;",
        "val Second=42; return Second;",
        "val a=42<Second>; return a.to_unit<Millis>()/1<Millis>;",
    ] {
        let out = compile(body);
        assert!(!out.status.success(), "{body}");
        assert!(out.stdout.is_empty(), "{body} emitted object");
    }
    fs::remove_dir_all(dir).unwrap();
}
