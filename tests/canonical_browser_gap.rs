//! Shared canonical browser source remains a native parity gap, never a pass.
use std::process::Command;
#[test]
fn canonical_browser_actor_has_no_silent_scalar_jit_fallback() {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/canonical-browser/actor-state.atto"
    );
    for mode in ["run", "ir"] {
        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .args([mode, file])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(
            result.stdout.is_empty(),
            "unsupported actor must publish no native result or IR"
        );
        assert!(String::from_utf8_lossy(&result.stderr).contains("DECL_KIND_UNSUPPORTED"));
    }
}
