//! Stage-0 Cranelift must reject ETS/pg/ActorGroup guest APIs before CLIF
//! emission. These are negative compiler gates, not runtime support.
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn check(source: &str, allowed: bool) {
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orescranelift-ets-pg-{}-{}.ores",
        std::process::id(),
        seq
    ));
    std::fs::write(&path, source).expect("write test source");
    for mode in ["run", "ir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg(mode)
            .arg(&path)
            .output()
            .expect("execute CLI");
        assert_eq!(
            output.status.success(),
            allowed,
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if !allowed {
            assert!(
                output.stdout.is_empty(),
                "{mode}: unsupported ETS/pg/ActorGroup source emitted executable output/CLIF"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                !stderr.is_empty() && !stderr.contains("panicked at"),
                "{mode}: missing compiler diagnostic or crash: {stderr}"
            );
        }
    }
    std::fs::remove_file(path).expect("unlink test source");
}

#[test]
fn source_ets_pg_and_group_ops_fail_closed() {
    for source in [
        "fnc main():int{return ets.new();}",
        "fnc main():int{return ets.lookup(1,2);}",
        "fnc main():int{return ets.compare_exchange_versioned(1,2,3);}",
        "fnc main():int{return pg.join(1,2);}",
        "fnc main():int{return pg.broadcast(1);}",
        "fnc main():int{return ActorGroup(1);}",
        "define actor Worker as end\nfnc main():int{return 42;}",
    ] {
        check(source, false);
    }
}

#[test]
fn mentions_in_comments_are_not_feature_admission() {
    check("fnc main():int{return 42;} // ets pg ActorGroup\n", true);
}
