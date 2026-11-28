//! Compiler-negative admission gate, not an HTTP runtime certificate.
#[allow(dead_code)]
#[path = "../src/frontend.rs"]
mod frontend;

#[test]
fn canonical_http_actor_declarations_fail_closed_until_transport_lowering_exists() {
    let cases = [
        include_str!("fixtures/http-actor-v1/serial-1x1.atto"),
        include_str!("fixtures/http-actor-v1/explicit-1x1.atto"),
        include_str!("fixtures/http-actor-v1/unsupported-3x4.atto"),
    ];
    for source in cases {
        assert!(
            frontend::parse(source).is_err(),
            "unsupported HTTP actor declaration entered scalar lowering: {source}"
        );
    }
    assert!(frontend::parse("fnc main():int{return 42;}").is_ok());
}
