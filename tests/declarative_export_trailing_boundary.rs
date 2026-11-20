#[path = "../src/frontend.rs"]
mod frontend;
use frontend::parse;

const MAIN: &str = "fnc main(): int { return 42; }";

#[test]
fn trailing_public_entrypoint_is_admitted_without_rewriting_program() {
    let no_export = parse(MAIN).unwrap();
    let trailing = parse(&format!("{MAIN}\nexport pub fnc main;")).unwrap();
    // The export declaration must not smuggle a runtime local into the
    // compiled scalar entrypoint. Assert this field in the standalone
    // frontend test crate as well as in the native backend.
    assert!(no_export.locals.is_empty());
    assert!(trailing.locals.is_empty());
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
