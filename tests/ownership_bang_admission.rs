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
        "ref! p to missing; unref! p; return 42;",
    ] {
        let source = format!("pub fnc main(): int {{ let a: int = 42; let mut RawPointer<int> p = RawPointer.empty<int>(); {body} }}");
        for mode in ["check", "run", "ir"] {
            let out = run_source(&source, mode);
            assert!(!out.status.success() && out.stdout.is_empty(), "{mode}: {body}");
        }
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
    assert!(clif.contains("stack_store")&&clif.contains("stack_load"),"{clif}");
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

    for (from,to) in [("unref! p;","p = null;"),("unref! p;",""),("free! s;",""),("free! s;","free! s; free! s;"),("(deref! p).x","(deref! p).z"),("ref! p to s;","ref!p to s;"),("ref! p to s;","ref!/*comment*/ p to s;"),("return first + second;","return p;"),("new Point(40, 1) at s;","new Point(9223372036854775807, 1) at s;"),("ref! p to s;","ref! p to s; ref! p to s;"),("unref! p;","unref! p; unref! p;"),("destroy! s;","destroy! s; destroy! s;")]{
        let bad=good.replacen(from,to,1);
        for mode in ["run","ir","check","emit-object"]{
            let out=run_source(&bad,mode);assert!(!out.status.success()&&out.stdout.is_empty(),"{mode}: {bad}");
        }
    }
}
