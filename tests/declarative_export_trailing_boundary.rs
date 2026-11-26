// Parser-only test crate intentionally imports a subset of the production frontend.
// Production binary retains its independent strict dead-code gate.
#[allow(dead_code)]
#[path = "../src/frontend.rs"]
mod frontend;
use frontend::parse;

const MAIN: &str = "fnc main(): int { return 42; }";

#[test]
fn trailing_public_entrypoint_is_admitted_without_rewriting_program() {
    let no_export = parse(MAIN).unwrap();
    let trailing = parse(&format!("{MAIN}\nexport pub fnc main;")).unwrap();
    assert_eq!(format!("{no_export:?}"), format!("{trailing:?}"));
}

#[test]
fn trailing_exports_reject_package_and_duplicate_authority() {
    for suffix in [
        "export pub[pkg] fnc main;",
        "export pub class main;",
        "export pub main;\nexport pub fnc main;",
        "export pub[friend] fnc main;",
        "export pub Missing;",
    ] {
        assert!(parse(&format!("{MAIN}\n{suffix}")).is_err(), "{suffix}");
    }
}
