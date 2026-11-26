//! Stage-0 Cranelift is scalar-only. Never compile unsupported async/actor
//! syntax into a scalar JIT value or silently interpret `await` as a no-op.
//! These are actual CLI source->parser->backend admission tests, not a
//! continuation implementation or a proof of LLVM actor-runtime parity.
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orescranelift-continuation-denial-{}-{}.ores",
            std::process::id(),
            n
        ));
        std::fs::write(&path, source).expect("write isolated source fixture");
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn run(mode: &str, source: &str) -> std::process::Output {
    let file = Fixture::new(source);
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&file.0)
        .output()
        .expect("invoke real Cranelift CLI")
}

#[test]
fn unsupported_continuations_fail_closed_in_both_jit_and_ir_modes() {
    // Keep this list deliberately broader than the currently implemented
    // scalar grammar. These must remain unsupported until fully checked
    // suspension frames, actor ownership and scheduler lowering exist.
    const UNSUPPORTED: &[&str] = &[
        "pub async fnc main():int{return 42;}",
        "fnc main():int{return await 42;}",
        "fnc main():int{return (await 1);}",
        "fnc main():int{return spawn Worker();}",
        "fnc main():int{return trap worker.send(1);}",
        "fnc main():int{return rt borrow x;}",
        "fnc main():int{return rt take x;}",
        "fnc main():int{return select 1 over on 1 { 42; } end;}",
        "fnc main():int{return nb select 1 over on 1 { 42; } end;}",
        "fnc main():int{return 1;} await another_task;",
        "define actor Worker as receive(int x)->void { } end fnc main():int{return 42;}",
        "fnc main():int{return actor.get_done();}",
    ];
    for (i, source) in UNSUPPORTED.iter().enumerate() {
        for mode in ["run", "ir"] {
            let out = run(mode, source);
            assert_eq!(
                out.status.code(),
                Some(1),
                "case {i} {mode}: unsupported source admitted"
            );
            assert!(
                out.stdout.is_empty(),
                "case {i} {mode}: emitted CLIF IR or guest value"
            );
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(
                err.contains("orescranelift:"),
                "case {i} {mode}: missing diagnostic: {err}"
            );
            assert!(
                !err.contains("panicked at"),
                "case {i} {mode}: compiler panic: {err}"
            );
        }
    }
}

#[test]
fn awaited_words_inside_comments_do_not_block_scalar_guest_execution() {
    let source = "fnc main():int{return 40+2;} // await spawn select actor rt borrow\n";
    let out = run("run", source);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"42\n");
}

#[test]
fn exact_mixed_counted_sources_and_contextual_headers_fail_closed() {
    let sources = [
        include_str!("fixtures/select-source-v2/counted-mixed-future-alias.atto"),
        include_str!("fixtures/select-source-v2/counted-multi-channel-fair.atto"),
        include_str!("fixtures/select-source-v2/counted-mixed-zero.atto"),
    ];
    for source in sources {
        for mode in ["run", "ir"] {
            let out = run(mode, source);
            assert_eq!(out.status.code(), Some(1), "{mode}: unsupported counted source admitted");
            assert!(out.stdout.is_empty(), "{mode}: emitted artifact");
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(err.contains("orescranelift:") && !err.contains("panicked at"), "{err}");
        }
    }
    for prefix in ["select", "nb select", "await select", "await nb select"] {
        for header in ["(0)", "<int>(3)", "(count)", "(true)"] {
            let source = format!("fnc main():int{{return {prefix}{header} over when readch c: int n -> {{ return n; }} end;}}");
            for mode in ["run", "ir"] {
                let out = run(mode, &source);
                assert_eq!(out.status.code(), Some(1), "{source}: {mode}");
                assert!(out.stdout.is_empty(), "{source}: {mode}: artifact");
                let err = String::from_utf8_lossy(&out.stderr);
                assert!(err.contains("orescranelift:") && !err.contains("panicked at"), "{err}");
            }
        }
    }
    assert_eq!(run("run", "fnc main():int{return 42;}").stdout, b"42\n");
}
