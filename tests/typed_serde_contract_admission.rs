//! Real compiler CLI admission, not serializer generation or AOT evidence.
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn invoke(mode: &str, src: &str) -> Output {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("atto-serde-admission-{}-{n}", std::process::id()));
    std::fs::create_dir(&dir).expect("exclusive fixture dir");
    let path = dir.join("main.atto");
    std::fs::write(&path, src).expect("write fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode).arg(&path).output().expect("execute compiler");
    std::fs::remove_dir_all(&dir).expect("remove fixture dir");
    result
}

#[test]
fn unsupported_nominal_serializers_cannot_publish_clif_or_guest_results() {
    for (kind, source) in [
        ("class", "define class User as pub to_json(): string { return \"x\"; } end pub fnc main(): int { return 0; }"),
        ("struct", "define struct User implements Serializable as value: int; end pub fnc main(): int { return 0; }"),
        ("interface", "define interface JsonSerializable as to_json(): string; end pub fnc main(): int { return 0; }"),
        ("trait", "define trait JsonDeserializable as from_json(string data): string; end pub fnc main(): int { return 0; }"),
    ] {
        for mode in ["run", "ir"] {
            let r = invoke(mode, source);
            assert_eq!(r.status.code(), Some(1), "{mode}/{kind}");
            assert!(r.stdout.is_empty(), "{mode}/{kind}: published output");
            let diag = String::from_utf8_lossy(&r.stderr);
            assert!(diag.contains(&format!("DECL_KIND_UNSUPPORTED: {kind}")), "{mode}/{kind}: {diag}");
        }
    }
}

#[test]
fn missing_serializer_name_is_not_an_implicit_builtin() {
    for mode in ["run", "ir"] {
        let r = invoke(mode, "pub fnc main(): int { return deserialize; }");
        assert_eq!(r.status.code(), Some(1));
        assert!(r.stdout.is_empty());
        let diag = String::from_utf8_lossy(&r.stderr);
        assert!(diag.contains("unbound or unsupported identifier"), "{diag}");
    }
}

#[test]
fn scalar_control_executes_and_produces_clif() {
    let src = "pub fnc main(): int { /* Serializable to_json */ return 42; }";
    let run = invoke("run", src);
    assert!(run.status.success(), "{:?}", run.stderr);
    assert_eq!(run.stdout, b"42\n");
    let ir = invoke("ir", src);
    assert!(ir.status.success(), "{:?}", ir.stderr);
    assert!(String::from_utf8_lossy(&ir.stdout).contains("function"));
}
