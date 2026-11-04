//! Actor Sync v1 is a negative-admission suite, not actor runtime parity.
//! Run through the actual bounded native compiler without clobbering /tmp.
#[path = "support/mod.rs"]
mod support;

fn invoke(mode: &str, source: &str) -> std::process::Output {
    support::run_source(mode, source.as_bytes(), &[])
}

#[test]
fn actor_sync_guest_source_is_not_silently_accepted_as_scalar_jit() {
    let cases = [
        (
            "isolated-actor",
            "define isolated actor Cart as state int count; end",
        ),
        (
            "actor-plus-main",
            "define isolated actor Cart as end fnc main():int{return 42;}",
        ),
        ("actor-state", "define actor Cart as state int count; end"),
        ("virtual-ref", "fnc main():int{return ActorRef<Cart>(0);}"),
        ("spawn", "fnc main():int{return spawn Cart();}"),
        ("watch", "fnc main():int{return watch(cart, 0);}"),
        ("snapshot", "fnc main():int{return snapshot(cart);}"),
        ("suspends", "suspends fnc main():int{return 42;}"),
        ("await", "fnc main():int{return await pending;}"),
        ("critical", "critical fnc main():int{return 42;}"),
        ("critical-block", "fnc main():int{critical {return 42;}}"),
        ("rt-borrow", "fnc main():int{return rt borrow 42;}"),
    ];
    for (label, source) in cases {
        for mode in ["run", "ir"] {
            let output = invoke(mode, source);
            assert!(
                !output.status.success(),
                "{mode} unexpectedly admitted actor-sync {label} as scalar program"
            );
            assert!(
                output.stdout.is_empty(),
                "{mode} generated native value / IR for unsupported {label}"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!stderr.is_empty(), "missing diagnostic for {label}");
            assert!(
                !stderr.to_lowercase().contains("panicked"),
                "{mode} panicked for {label}: {stderr}"
            );
        }
    }
}

#[test]
fn actor_sync_words_in_comments_do_not_break_supported_scalar_guest() {
    let output = invoke(
        "run",
        "fnc main():int{return 42;} // actor sync watch critical\n",
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
}
