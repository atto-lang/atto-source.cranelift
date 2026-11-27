//! Real Cranelift scalar CLI; unsupported ownership is denied before native JIT.
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

// CI may run tests concurrently in a shared temporary directory. Never
// overwrite another process's fixture (or follow an attacker-owned symlink).
struct SourceFixture {
    path: PathBuf,
}

impl SourceFixture {
    fn create(source: &str) -> Self {
        for _ in 0..64 {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ores-ownership-bang-{}-{sequence}.atto",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let fixture = Self { path };
                    file.write_all(source.as_bytes())
                        .expect("write exclusive ownership fixture");
                    file.sync_all().expect("sync exclusive ownership fixture");
                    return fixture;
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create exclusive ownership fixture: {error}"),
            }
        }
        panic!("could not allocate an exclusive ownership fixture after 64 attempts");
    }
}

impl Drop for SourceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn run_source(source: &str, mode: &str) -> Output {
    let fixture = SourceFixture::create(source);
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&fixture.path)
        .output()
        .expect("run Cranelift ownership fixture")
}

#[test]
fn rejects_ownership_prefixes_before_run_or_ir() {
    for expression in [
        "copy!42",
        "take!42",
        "borrow! value",
        "borrow!mut value",
        "borrow! mut value",
        "mut borrow! value",
        "mut borrow!value",
        "share!value",
        "ref!value",
        "ref!(value)",
        "deref!handle",
        "deref! handle",
        "unref!handle",
        "unref! handles[0]",
        "ref!eqfoo",
        "freeze!value",
        "weak!value",
        "copy!!value",
        "borrow!\nmut value",
        "borrow!/* checked loan */mut value",
        "mut borrow!\nvalue",
    ] {
        for mode in ["check", "run", "ir"] {
            let source = format!("fnc main():int{{return {expression};}}");
            let output = run_source(&source, mode);
            assert!(!output.status.success(), "{mode} accepted {expression}");
            assert!(
                output.stdout.is_empty(),
                "{mode} emitted output for {expression}"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(
                    if expression.split_once('!').is_some_and(|(_, tail)| {
                        let tail = tail.strip_prefix('!').unwrap_or(tail);
                        !tail
                            .as_bytes()
                            .first()
                            .is_some_and(|c| b" \t\r\n\x0b\x0c".contains(c))
                    }) {
                        "E_BANG_KEYWORD_WHITESPACE"
                    } else {
                        "OWNERSHIP_BANG_BACKEND_UNSUPPORTED"
                    }
                ),
                "{mode} wrong diagnostic for {expression}: {stderr}"
            );
        }
    }
}

#[test]
fn commented_ownership_does_not_poison_scalar_execution() {
    for mode in ["check", "run", "ir"] {
        let output = run_source("fnc main():int{// borrow! mut item\nreturn 42;}", mode);
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if mode == "check" {
            assert!(output.stdout.is_empty(), "check emitted output");
        } else {
            assert!(!output.stdout.is_empty(), "{mode} produced no output");
        }
    }
}

#[test]
fn checked_handle_aliases_do_not_confuse_comparison_or_spaced_punctuation() {
    // A whitespace gap before ! breaks the contextual operator. Likewise,
    // != and !eq must not be reclassified as ownership/checked-handle effects.
    // The scalar parser may reject these for other reasons; this test only
    // asserts that it cannot misreport a typed ownership operation.
    for expression in [
        "ref !handle",
        "deref !handle",
        "unref !handle",
        "ref!=handle",
        "deref!eq handle",
        "unref!eq handle",
        "copy!eq handle",
        "mut borrow!eq handle",
    ] {
        for mode in ["check", "run", "ir"] {
            let source = format!("fnc main():int{{return {expression};}}");
            let output = run_source(&source, mode);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                !stderr.contains("OWNERSHIP_BANG_BACKEND_UNSUPPORTED"),
                "{mode} misclassified {expression}: {stderr}"
            );
        }
    }
}

#[test]
fn checked_handle_spelling_inside_comments_is_inert() {
    for mode in ["check", "run", "ir"] {
        let output = run_source(
            "fnc main():int{/* ref!item, deref!handle, unref!handle */return 42;}",
            mode,
        );
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn nested_comments_are_inert_and_nesting_limit_is_fail_closed() {
    for mode in ["check", "run", "ir"] {
        let benign = "fnc main():int{/* outer /* ref!bad weak!bad */ still outer */return 42;}";
        let valid = run_source(benign, mode);
        assert!(
            valid.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&valid.stderr)
        );
        let overdeep = format!(
            "fnc main():int{{{}42{}return 42;}}",
            "/*".repeat(129),
            "*/".repeat(129)
        );
        let denied = run_source(&overdeep, mode);
        assert!(
            !denied.status.success(),
            "{mode} accepted 129 nested comments"
        );
        assert!(
            denied.stdout.is_empty(),
            "{mode} emitted output for overdeep comment"
        );
        assert!(
            String::from_utf8_lossy(&denied.stderr).contains("block comment nesting limit"),
            "{mode}: {}",
            String::from_utf8_lossy(&denied.stderr)
        );
    }
}

#[test]
fn exclusive_fixture_is_removed_on_scope_exit() {
    let path = {
        let fixture = SourceFixture::create("fnc main():int{return 42;}");
        assert!(fixture.path.is_file());
        assert_eq!(
            std::fs::read_to_string(&fixture.path).expect("read exclusive fixture"),
            "fnc main():int{return 42;}"
        );
        fixture.path.clone()
    };
    assert!(!path.exists(), "exclusive fixture leaked after use");
}

#[test]
fn exact_separator_matrix_denies_before_check_run_or_ir() {
    for keyword in [
        "trap!", "throw!", "raise!", "panic!", "copy!", "copy!!", "take!", "move!", "borrow!",
        "share!", "pin!", "drop!", "pipe!", "rpipe!", "stop!", "expand!", "ref!", "deref!",
        "unref!", "freeze!", "weak!",
    ] {
        for tail in ["value", "(42)", "/*comment*/ value", "", "\u{a0}value"] {
            for mode in ["check", "run", "ir"] {
                let source = format!("fnc main():int{{return {keyword}{tail};}}");
                let out = run_source(&source, mode);
                assert!(
                    !out.status.success() && out.stdout.is_empty(),
                    "{mode}: {source}"
                );
                let err = String::from_utf8_lossy(&out.stderr);
                assert!(
                    err.contains("E_BANG_KEYWORD_WHITESPACE"),
                    "{mode}: {source}: {err}"
                );
            }
        }
    }
    for gap in [" ", "\t", "\r", "\n", "\x0b", "\x0c"] {
        let out = run_source(
            &format!("fnc main():int{{return move!{gap}value;}}"),
            "check",
        );
        assert!(!out.status.success() && out.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("OWNERSHIP_BANG_BACKEND_UNSUPPORTED")
        );
    }
}

#[test]
fn bounded_reusable_slots_native_cli() {
    let good = [
        "pub fnc main(): int { let a: int = 40; let b: int = 2; let mut RawPointer<int> p = RawPointer.empty<int>(); ref! p to a; val first = deref! p; unref! p; ref! p to b; val second = deref! p; unref! p; return first + second; }",
        "pub fnc main(): int { let mut RawPointer<int> p = RawPointer.empty<int>(); return 42; }",
        "pub fnc main(): int { let a: int = 42; let mut RawPointer<int> p = RawPointer.empty<int>(); ref! p to a; unref! p; return a; }",
    ];
    for source in good {
        let run = run_source(source, "run");
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
        let ir = run_source(source, "ir");
        assert!(ir.status.success() && !ir.stdout.is_empty());
    }
    for body in [
        "val x = deref! p; return x;",
        "p = null; return 42;",
        "ref! p to a; unref! p; unref! p; return 42;",
        "ref! p to a; ref! p to a; unref! p; return 42;",
        "ref! p to a; return 42;",
        "return p;",
        "ref ! p to a; unref! p; return 42;",
        "ref!p to a; unref! p; return 42;",
        "ref! p to a; val x = deref!p; unref! p; return x;",
        "ref! p to a; unref!p; return 42;",
        "ref! p to missing; unref! p; return 42;",
    ] {
        let source = format!("pub fnc main(): int {{ let a: int = 42; let mut RawPointer<int> p = RawPointer.empty<int>(); {body} }}");
        for mode in ["check", "run", "ir"] {
            let out = run_source(&source, mode);
            assert!(!out.status.success() && out.stdout.is_empty(), "{mode}: {body}");
        }
    }
}

#[cfg(all(unix, target_pointer_width = "64"))]
#[test]
fn bounded_reusable_slots_emit_executable_native_objects() {
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = (0..64)
        .find_map(|_| {
            let seq = NEXT.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("atto-slot-aot-{}-{seq}", std::process::id()));
            match std::fs::create_dir(&p) {
                Ok(()) => Some(p),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => None,
                Err(e) => panic!("cannot create native fixture directory: {e}"),
            }
        })
        .expect("exclusive fixture directory");
    let _cleanup = Cleanup(dir.clone());
    let object = dir.join("guest.o");
    let harness = dir.join("host.c");
    std::fs::write(&harness, r#"
#include <stdint.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void) {
    int64_t out = -123;
    assert(oreslang_scalar_entry(0,0,0,0,&out) == 0);
    assert(out == 42);
    return 0;
}
"#).unwrap();
    let good = "pub fnc main(): int { let a: int = 40; let b: int = 2; let mut RawPointer<int> p = RawPointer.empty<int>(); ref! p to a; val first = deref! p; unref! p; ref! p to b; val second = deref! p; unref! p; return first + second; }";
    let emitted = run_source(good, "emit-object");
    assert!(emitted.status.success(), "{}", String::from_utf8_lossy(&emitted.stderr));
    assert!(!emitted.stdout.is_empty());
    std::fs::write(&object, emitted.stdout).unwrap();
    for opt in ["-O0", "-O2"] {
        let binary = dir.join(format!("guest{opt}"));
        let linked = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Werror", "-fsanitize=undefined", "-fno-sanitize-recover=all", opt])
            .arg(&harness).arg(&object).arg("-o").arg(&binary).output().unwrap();
        assert!(linked.status.success(), "{}", String::from_utf8_lossy(&linked.stderr));
        let ran = Command::new(binary).output().unwrap();
        assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    }
    for body in [
        "val x = deref! p; return x;",
        "ref! p to a; return 42;",
        "ref! p to a; unref! p; unref! p; return 42;",
        "return p;",
    ] {
        let source = format!("pub fnc main(): int {{ let a: int = 42; let mut RawPointer<int> p = RawPointer.empty<int>(); {body} }}");
        let denied = run_source(&source, "emit-object");
        assert!(!denied.status.success() && denied.stdout.is_empty(), "{body}");
    }
}

#[test]
fn placement_point_native_memory_and_rejections(){
    let good="// expect-exit: 42\ndefine struct Point { int x; int y; } with { constructor(int x, int y) { self.x = x; self.y = y; } } pub fnc main(): int { let Storage<Point> s = alloc! Point; let mut RawPointer<Point> p = RawPointer.empty<Point>(); new Point(40, 1) at s; ref! p to s; val first = (deref! p).x; unref! p; destroy! s; new Point(99, 2) at s; ref! p to s; val second = (deref! p).y; unref! p; destroy! s; free! s; return first + second; }\n";
    let run=run_source(good,"run");
    assert!(run.status.success(),"{}",String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(),"42");
    let ir=run_source(good,"ir");
    assert!(ir.status.success());
    let clif=String::from_utf8_lossy(&ir.stdout);
    // Current CLIF builder canonicalizes stack_load/store into stack_addr
    // plus nontrapping memory operations. Require the concrete slot and
    // both field addresses, not the spelling of a pre-canonicalized opcode.
    assert!(clif.contains("explicit_slot 16, align = 8"),"{clif}");
    assert!(clif.contains("stack_addr.i64 ss0")&&clif.contains("stack_addr.i64 ss0+8"),"{clif}");
    assert!(clif.matches("store notrap").count()>=8 && clif.matches("load.i64 notrap").count()>=2,"{clif}");
    let object=run_source(good,"emit-object");
    assert!(object.status.success()&&!object.stdout.is_empty());
    // An emitted object is not evidence of native execution by itself.
    // Link an observer that checks the full signed result and repeatability.
    #[cfg(all(unix,target_pointer_width="64"))]
    {
        let second=run_source(good,"emit-object");
        assert!(second.status.success());
        assert_eq!(object.stdout,second.stdout);
        let dir=std::env::temp_dir().join(format!("atto-placement-link-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir(&dir).expect("exclusive native link directory");
        struct Cleanup(PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
        let _cleanup=Cleanup(dir.clone());
        let obj=dir.join("guest.o");let host=dir.join("host.c");let bin=dir.join("guest");
        std::fs::write(&obj,&object.stdout).unwrap();
        std::fs::write(&host,r#"
#include <stdint.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void){
    for(int i=0;i<100;i++){
        int64_t out=-17;
        if(oreslang_scalar_entry(0,0,0,0,&out)!=0||out!=42)return 1;
    }
    return 0;
}
"#).unwrap();
        for opt in ["-O0","-O2"] {
            let link=Command::new("cc").args(["-std=c11","-Wall","-Wextra","-Werror",opt])
                .arg(&host).arg(&obj).arg("-o").arg(&bin).output().unwrap();
            assert!(link.status.success(),"{:?}",link);
            assert!(Command::new(&bin).status().unwrap().success());
            println!("POINT_PLACEMENT_LINKED_NATIVE_PASS result=42 repetitions=100 observer_opt={opt}");
        }
    }

    for (from,to) in [("unref! p;","p = null;"),("self.y = y;","self.x = y;"),("self.y = y;",""),("int y;","int x;"),("new Point(40, 1) at s;","new Point(40, 1);"),("unref! p;",""),("free! s;",""),("free! s;","free! s; free! s;"),("(deref! p).x","(deref! p).z"),("ref! p to s;","ref!p to s;"),("ref! p to s;","ref!/*comment*/ p to s;"),("return first + second;","return p;"),("new Point(40, 1) at s;","new Point(9223372036854775807, 1) at s;"),("ref! p to s;","ref! p to s; ref! p to s;"),("unref! p;","unref! p; unref! p;"),("destroy! s;","destroy! s; destroy! s;")]{
        let bad=good.replacen(from,to,1);
        for mode in ["run","ir","check","emit-object"]{
            let out=run_source(&bad,mode);assert!(!out.status.success()&&out.stdout.is_empty(),"{mode}: {bad}");
        }
    }
}

#[test]
fn source_defined_mixed_width_placement() {
    let good="define struct Packet { i8 tag; int count; i16 tail; } with { constructor(i8 tag, int count, i16 tail) { self.tag = tag; self.count = count; self.tail = tail; } } pub fnc main(): int { let Storage<Packet> s = alloc! Packet; let mut RawPointer<Packet> p = RawPointer.empty<Packet>(); new Packet(-3, 40, 5) at s; ref! p to s; val a = (deref! p).tag; val b = (deref! p).count; val c = (deref! p).tail; unref! p; destroy! s; free! s; return a + b + c; }";
    let out=run_source(good,"run");
    assert!(out.status.success(),"{:?}",out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(),"42");
    let ir=run_source(good,"ir");assert!(ir.status.success(),"{:?}",ir);
    let clif=String::from_utf8_lossy(&ir.stdout);
    assert!(clif.contains("explicit_slot 24, align = 8"),"{clif}");
    assert!(clif.contains("stack_addr.i64 ss0+8")&&clif.contains("stack_addr.i64 ss0+16"),"{clif}");
    assert!(clif.contains("sextend.i64"),"{clif}");
    #[cfg(all(unix,target_pointer_width="64"))]
    {
        let dir=std::env::temp_dir().join(format!("atto-mixed-placement-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let _cleanup=Cleanup(dir.clone());
        let obj=dir.join("guest.o");let host=dir.join("host.c");let bin=dir.join("guest");
        let object=run_source(good,"emit-object");assert!(object.status.success()&&!object.stdout.is_empty());
        std::fs::write(&obj,object.stdout).unwrap();
        std::fs::write(&host,r#"
#include <stdint.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void){for(int i=0;i<100;i++){int64_t out=-1;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==42);}return 0;}
"#).unwrap();
        for opt in ["-O0","-O2"] {
            let link=Command::new("cc").args(["-std=c11","-Wall","-Wextra","-Werror",opt])
                .arg(&host).arg(&obj).arg("-o").arg(&bin).output().unwrap();
            assert!(link.status.success(),"{:?}",link);
            assert!(Command::new(&bin).status().unwrap().success());
            println!("SOURCE_MIXED_PLACEMENT_NATIVE_PASS repetitions=100 observer_opt={opt}");
        }
    }
    for (from,to) in [
        ("-3, 40, 5","128, 40, 5"),("-3, 40, 5","-129, 40, 5"),
        ("-3, 40, 5","-3, 40, 32768"),("Storage<Packet>","Storage<Other>"),
        ("RawPointer.empty<Packet>","RawPointer.empty<Other>"),
        ("i16 tail;","i16 tag;"),("self.tail = tail;","self.tag = tail;"),
        ("(deref! p).tail","(deref! p).unknown"),("new Packet(","new Other("),
        ("i8 tag, int count","i16 tag, int count"),("new Packet(-3, 40, 5)","new Packet(-3, 40)")
    ] {
        let bad=good.replacen(from,to,1);assert_ne!(good,bad);
        for mode in ["check","run","ir","emit-object"] {
            let r=run_source(&bad,mode);assert!(!r.status.success()&&r.stdout.is_empty(),"{mode}: {bad}");
        }
    }
}

#[test]
fn source_defined_nested_placement() {
    let good="define struct Payload { int count; i8 flag; } with { constructor(int count, i8 flag) { self.count = count; self.flag = flag; } } define struct Packet { i8 tag; Payload pair; i16 tail; } with { constructor(i8 tag, Payload pair, i16 tail) { self.tag = tag; self.pair = pair; self.tail = tail; } } pub fnc main(): int { let Storage<Packet> s = alloc! Packet; let mut RawPointer<Packet> p = RawPointer.empty<Packet>(); new Packet(-3, Payload(40, -2), 7) at s; ref! p to s; val a = (deref! p).tag; val b = (deref! p).pair.count; val c = (deref! p).pair.flag; val d = (deref! p).tail; unref! p; destroy! s; free! s; return a + b + c + d; }";
    let out=run_source(good,"run");assert!(out.status.success(),"{:?}",out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(),"42");
    let ir=run_source(good,"ir");assert!(ir.status.success(),"{:?}",ir);
    let clif=String::from_utf8_lossy(&ir.stdout);
    assert!(clif.contains("explicit_slot 32, align = 8"),"{clif}");
    for offset in [8,16,24]{assert!(clif.contains(&format!("stack_addr.i64 ss0+{offset}")),"{clif}");}
    #[cfg(all(unix,target_pointer_width="64"))]
    {
        let dir=std::env::temp_dir().join(format!("atto-nested-placement-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let _cleanup=Cleanup(dir.clone());
        let obj=dir.join("guest.o");let host=dir.join("host.c");let bin=dir.join("guest");
        let object=run_source(good,"emit-object");assert!(object.status.success()&&!object.stdout.is_empty(),"{:?}",object);
        std::fs::write(&obj,object.stdout).unwrap();
        std::fs::write(&host,r#"
#include <stdint.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void){for(int i=0;i<100;i++){int64_t out=-1;assert(oreslang_scalar_entry(0,0,0,0,&out)==0);assert(out==42);}return 0;}
"#).unwrap();
        for opt in ["-O0","-O2"] {
            let link=Command::new("cc").args(["-std=c11","-Wall","-Wextra","-Werror",opt]).arg(&host).arg(&obj).arg("-o").arg(&bin).output().unwrap();
            assert!(link.status.success(),"{:?}",link);
            assert!(Command::new(&bin).status().unwrap().success());
            println!("SOURCE_NESTED_PLACEMENT_NATIVE_PASS repetitions=100 observer_opt={opt}");
        }
    }
    for (from,to) in [
        ("Payload pair;","Packet pair;"),("Payload pair;","Missing pair;"),
        ("Payload(40, -2)","Other(40, -2)"),("Payload(40, -2)","Payload(40, 128)"),
        (".pair.count;",".pair;"),(".pair.flag;",".pair.missing;"),
        (".pair.count;",".tag.count;"),("self.pair = pair;","self.pair = tail;"),
        ("Payload pair, i16 tail","int pair, i16 tail"),("Payload(40, -2)","Payload(40)")
    ]{
        let bad=good.replacen(from,to,1);assert_ne!(bad,good);
        for mode in ["check","run","ir","emit-object"]{
            let r=run_source(&bad,mode);assert!(!r.status.success()&&r.stdout.is_empty(),"{mode}: {bad}");
        }
    }
}

#[test]
fn typed_runtime_placement_arguments_and_atomic_failure() {
    let good="define struct Payload { int count; i8 flag; } with { constructor(int count, i8 flag) { self.count = count; self.flag = flag; } } define struct Packet { i8 tag; Payload payload; i16 tail; } with { constructor(i8 tag, Payload payload, i16 tail) { self.tag = tag; self.payload = payload; self.tail = tail; } } pub fnc main(i8 tag_in, int count_in, i8 flag_in, i16 tail_in): int { let Storage<Packet> s = alloc! Packet; let mut RawPointer<Packet> p = RawPointer.empty<Packet>(); new Packet(tag_in, Payload(count_in, flag_in), tail_in) at s; ref! p to s; val a = (deref! p).tag; val b = (deref! p).payload.count; val c = (deref! p).payload.flag; val d = (deref! p).tail; unref! p; destroy! s; free! s; return a + b + c + d; }";
    let fixture=SourceFixture::create(good);
    for (args,expected) in [(["-3","40","-2","7"],Some("42")),(["0","-10","1","2"],Some("-7")),(["128","40","-2","7"],None),(["0","9223372036854775807","1","0"],None)]{
        let out=Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("run").arg(&fixture.path).args(args).output().unwrap();
        if let Some(n)=expected{assert!(out.status.success(),"{:?}",out);assert_eq!(String::from_utf8_lossy(&out.stdout).trim(),n)}
        else{assert!(!out.status.success()&&out.stdout.is_empty(),"{:?}",out)}
    }
    assert!(!run_source(good,"run").status.success());
    let ir=run_source(good,"ir");assert!(ir.status.success(),"{:?}",ir);
    let clif=String::from_utf8_lossy(&ir.stdout);assert!(clif.contains("sadd_overflow")&&clif.contains("ireduce"),"{clif}");
    #[cfg(all(unix,target_pointer_width="64"))] {
        let dir=std::env::temp_dir().join(format!("atto-runtime-placement-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        struct Cleanup(PathBuf);impl Drop for Cleanup{fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let _cleanup=Cleanup(dir.clone());
        let obj=dir.join("guest.o");let host=dir.join("host.c");let bin=dir.join("guest");
        let object=run_source(good,"emit-object");assert!(object.status.success()&&!object.stdout.is_empty(),"{:?}",object);
        std::fs::write(&obj,object.stdout).unwrap();
        std::fs::write(&host,r#"
#include <stdint.h>
#include <assert.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void){for(int i=0;i<100;i++){
  int64_t out=-999;
  assert(oreslang_scalar_entry(-3,40,-2,7,&out)==0 && out==42);
  assert(oreslang_scalar_entry(0,-10,1,2,&out)==0 && out==-7);
  out=-999;
  assert(oreslang_scalar_entry(128,40,-2,7,&out)==1 && out==-999);
  assert(oreslang_scalar_entry(0,40,-129,7,&out)==1 && out==-999);
  assert(oreslang_scalar_entry(0,40,1,32768,&out)==1 && out==-999);
  assert(oreslang_scalar_entry(0,INT64_MAX,1,0,&out)==1 && out==-999);
  assert(oreslang_scalar_entry(-1,INT64_MIN,0,0,&out)==1 && out==-999);
}return 0;}
"#).unwrap();
        for opt in ["-O0","-O2"]{
            let link=Command::new("cc").args(["-std=c11","-Wall","-Wextra","-Werror",opt]).arg(&host).arg(&obj).arg("-o").arg(&bin).output().unwrap();
            assert!(link.status.success(),"{:?}",link);assert!(Command::new(&bin).status().unwrap().success());
            println!("RUNTIME_PLACEMENT_NATIVE_PASS success=200 failure=500 observer_opt={opt}");
        }
    }
    for (a,b) in [("i8 tag_in","int tag_in"),("i8 flag_in","i8 tag_in"),("Payload(count_in, flag_in)","Payload(count_in, missing)"),("new Packet(tag_in,","new Packet(count_in,"),("val a =","val tag_in =")]{
        let bad=good.replacen(a,b,1);assert_ne!(good,bad);
        for mode in ["check","ir","emit-object"]{let r=run_source(&bad,mode);assert!(!r.status.success()&&r.stdout.is_empty(),"{mode}: {bad}");}
    }
}

#[test]
fn local_pair_const_native_cli() {
    for body in [
        "const x:=42;const Pair<int> p=x;return p.value;",
        "const x=x:=42;const Pair<int> p=x;return p.value;",
        "const x=Pair.of(\"x\",42);const Pair<int> p=x;return p.value;",
        "const x=\"x\":=42;return x.value;",
        "const x=y:=42;return x.value;",
        "const x:=40;const y:=x.value+2;return y.value;",
    ] {
        let source = format!("pub fnc main():int{{{body}}}");
        let output = run_source(&source,"run");
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
        let ir = run_source(&source,"ir");
        assert!(ir.status.success(), "{source}: {}", String::from_utf8_lossy(&ir.stderr));
        assert!(!ir.stdout.is_empty());
    }
    for body in [
        "const x:=42;return x;",
        "const x:=42;return x.key;",
        "const x:=42;const x:=1;return x.value;",
        "const x=x:=x;return x.value;",
        "const x:=x;return x.value;",
        "const x:=true;return x.value;",
        "const x: =42;return x.value;",
        "const x:=42;x.value=7;return x.value;",
        "const int x=42;const Pair<int> p=x;return 0;",
        "const x=Pair.of(7,42);return x.value;",
        "const x:=9223372036854775807+1;return x.value;",
        "val y=42;const x:=y;return x.value;",
    ] {
        let source = format!("pub fnc main():int{{{body}}}");
        for mode in ["check","run","ir"] {
            let output = run_source(&source,mode);
            assert!(!output.status.success(), "{mode}: {source}");
            assert!(output.stdout.is_empty(), "{mode}: {source}");
        }
    }
}
