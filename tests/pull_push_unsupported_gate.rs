//! Cranelift scalar JIT cannot lower actor subscriptions, channels or
//! async. Unsupported source must fail parsing; no silent scalar fallback.
#[path = "../src/frontend.rs"]
mod frontend;

#[test]
fn pull_push_remains_fail_closed_until_native_lowering_exists() {
    let unsupported = [
        "pub async fnc main():int{return 1;}",
        "fnc main():int { val Channel<int> output = Channel.new<int>(1); return 0; }",
        "fnc main():int { return Flow.of(1).next(); }",
        "fnc main():int { nb writech output, 1; return 0; }",
        "fnc main():int { return await input.next(); }",
        "fnc main():int { return rt take owner; }",
    ];
    for source in unsupported {
        assert!(
            frontend::parse(source).is_err(),
            "Cranelift parser silently accepted unsupported pull/push: {source}"
        );
    }
    let accepted = frontend::parse("fnc main():int{return 42;}")
        .expect("negative gate must preserve scalar JIT admission");
    assert!(accepted.locals.is_empty(), "scalar entry must not gain hidden runtime locals");
}
