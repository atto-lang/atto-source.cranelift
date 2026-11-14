use orescranelift::frontend::parse;
const MAIN: &str = "fnc main(): int { return 42; }";
#[test]
fn top_and_bottom_file_export_parse() {
    assert!(parse(&format!("export pub main;\n{MAIN}")).is_ok());
    assert!(parse(&format!("{MAIN}\nexport pub fnc main;")).is_ok());
}
#[test]
fn unsupported_export_targets_fail_closed() {
    for src in [
        format!("export pub[pkg] fnc main;\n{MAIN}"),
        format!("export pub class Foo;\n{MAIN}"),
        format!("export pub Missing;\n{MAIN}"),
        format!("export pub main;\nexport pub fnc main;\n{MAIN}"),
        format!("export pub main;\n{MAIN}\nexport pub main;"),
        format!("export pub[friend] fnc main;\n{MAIN}"),
    ] {
        assert!(parse(&src).is_err(), "unexpected acceptance: {src}");
    }
}
