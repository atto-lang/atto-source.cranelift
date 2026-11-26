// Concrete compiler-front-end admission, not a README or file-existence pass.
// This directly compiles the real backend parser (included by path for a
// dependency-free rustc gate), not a surrogate regex source scanner.
// Parser-only test crate intentionally imports a subset of the production frontend.
// Production binary retains its independent strict dead-code gate.
#[allow(dead_code)]
#[path = "../src/frontend.rs"]
mod frontend;

const TIME: &str = include_str!("../stdlib/std/time.atto");

#[test]
fn std_time_class_and_module_are_not_silently_accepted_as_native_jit() {
    assert!(TIME.contains("define module time as"));
    assert!(TIME.contains("define pub class Duration as"));
    assert!(TIME.contains("pub fnc manual_clock("));
    let rejected = frontend::parse(TIME)
        .expect_err("class/time ABI is not in the current Cranelift scalar compiler");
    assert!(!rejected.is_empty());
}

#[test]
fn std_time_import_and_sleep_are_not_compiled_without_runtime_lowering() {
    for guest in [
        "import module time from \"std/time\"; fnc main():int{return 42;}",
        "fnc main():int{await time.sleep(100);return 42;}",
        "fnc main():int{return time.seconds(1);}",
    ] {
        let e = frontend::parse(guest)
            .expect_err("unsupported hosted time must fail closed at native compiler admission");
        assert!(!e.is_empty());
    }
}

#[test]
fn scalar_frontend_still_admits_existing_compiled_guest_control() {
    let program = frontend::parse("fnc main():int{return 42;}")
        .expect("existing scalar JIT syntax must remain available");
    assert!(matches!(program.result, frontend::Expr::Literal(42)));
}
