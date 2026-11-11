use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn execute(source: &str, mode: &str) -> std::process::Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "atto-cranelift-match-tilde-{}-{id}.ores", std::process::id()
    ));
    std::fs::write(&path, source.as_bytes()).expect("write test fixture");
    let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(&path)
        .output()
        .expect("run actual orescranelift executable");
    std::fs::remove_file(&path).expect("remove test fixture");
    result
}

#[test]
fn match_and_tilde_fail_before_jit_execution_or_clif_publication() {
    let cases = [
        ("fnc main():int{return match<int> 9 over on 1 -> {11;} default -> {42;} end;}",
         "MATCH_DEFAULT_BACKEND_UNSUPPORTED"),
        ("fnc main():int{return match 9 over default -> {42;} end;}",
         "MATCH_DEFAULT_BACKEND_UNSUPPORTED"),
        ("fnc main():int{return match 9 over on default -> {42;} end;}",
         "MATCH_DEFAULT_BACKEND_UNSUPPORTED"),
        ("fnc main():int{return ~map<string,int>(\"a\" => 42);}",
         "TILDE_AGGREGATE_BACKEND_UNSUPPORTED"),
        ("fnc main():int{return ~record(foo:42);}",
         "TILDE_AGGREGATE_BACKEND_UNSUPPORTED"),
        ("fnc main():int{return ~(foo:42);}",
         "TILDE_AGGREGATE_BACKEND_UNSUPPORTED"),
        ("fnc main():int{return ~();}",
         "TILDE_AGGREGATE_BACKEND_UNSUPPORTED"),
    ];
    for (source, diagnostic) in cases {
        for mode in ["run", "ir"] {
            let result = execute(source, mode);
            assert!(!result.status.success(), "unsupported source admitted in {mode}: {source}");
            assert!(result.stdout.is_empty(), "unproved source emitted result/CLIF");
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(stderr.contains(diagnostic) && stderr.contains("byte"),
                "unexpected diagnostic: {stderr}");
        }
    }
}

#[test]
fn actual_scalar_cranelift_guest_still_returns_42_with_inert_comments() {
    let input = "fnc main():int{// match 9 over ~map()\nreturn 40 + 2;}";
    let result = execute(input, "run");
    assert!(result.status.success(), "valid source failed: {}",
        String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "42");
    let ir = execute(input, "ir");
    assert!(ir.status.success(), "valid scalar CLIF failed: {}",
        String::from_utf8_lossy(&ir.stderr));
    assert!(!ir.stdout.is_empty(), "valid scalar guest emitted no IR");
}
