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
    let repeated = parse(&format!("export pub main;\n{MAIN}\nexport pub fnc main;")).unwrap();
    assert_eq!(format!("{no_export:?}"), format!("{repeated:?}"));
}

#[test]
fn trailing_exports_reject_package_and_conflicting_authority() {
    for suffix in [
        "export pub[pkg] fnc main;",
        "export pub class main;",
        "export pub main;\nexport pub[pkg] fnc main;",
        "export pub[friend] fnc main;",
        "export pub Missing;",
    ] {
        assert!(parse(&format!("{MAIN}\n{suffix}")).is_err(), "{suffix}");
    }
}

#[test]
fn raw_export_directive_budget_is_enforced() {
    assert!(parse(&format!("{}{MAIN}", "export pub main;\n".repeat(128))).is_ok());
    assert!(
        parse(&format!(
            "{}{MAIN}\nexport pub main;",
            "export pub main;\n".repeat(128)
        ))
        .unwrap_err()
        .contains("EXPORT_SOURCE_LIMIT")
    );
}
