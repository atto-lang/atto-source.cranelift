//! Continuations remain unsupported by the stage-0 native scalar compiler.
//! A rejection is not proof of async/actor implementation parity.
#[path = "support/mod.rs"]
mod support;

fn run(mode: &str, source: &str) -> std::process::Output {
    support::run_source(mode, source.as_bytes(), &[])
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
