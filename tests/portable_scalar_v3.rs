//! v3: real Cranelift CLI/JIT execution on 12 positive + 9 negative sources.
//! Source manifest is shared byte-for-byte; source-only inclusion is not an executed pass.
use std::fs::{self,OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64,Ordering};

const MANIFEST: &str=include_str!("../contracts/parity/portable-scalar-v3-20261101.json");
static NEXT:AtomicU64=AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new()->Self {
        let n=NEXT.fetch_add(1,Ordering::Relaxed);
        let dir=std::env::temp_dir().join(format!("orescranelift-v3-{}-{n}",std::process::id()));
        fs::create_dir(&dir).expect("fixture directory collision or creation failure");
        Self(dir)
    }
}
impl Drop for Fixture { fn drop(&mut self){let _=fs::remove_dir_all(&self.0);} }
fn run_one(f:&Fixture,id:&str,expr:&str)->std::process::Output {
    assert!(MANIFEST.contains(&format!("\"id\": \"{id}\"")),"missing manifest id {id}");
    assert!(MANIFEST.contains(&format!("\"expression\": \"{expr}\"")),"manifest expression drift for {id}");
    let path=f.0.join(format!("{id}.ores"));
    let mut file=OpenOptions::new().write(true).create_new(true).open(&path).expect("fixture must be new");
    writeln!(file,"pub fnc main():int{{return {expr};}}").expect("write guest source");
    file.flush().expect("flush guest source");
    drop(file);
    Command::new(env!("CARGO_BIN_EXE_orescranelift")).arg("run").arg(&path).output()
        .expect("real Cranelift CLI must be available")
}
#[test]
fn v3_inventory_is_exact_and_fail_closed() {
    assert!(MANIFEST.contains("\"schema_version\": \"2026-11-01.scalar-v3\""));
    assert_eq!(MANIFEST.matches("\"id\":").count(),21);
    assert_eq!(MANIFEST.matches("\"expression\":").count(),21);
    assert_eq!(MANIFEST.matches("\"expected\":").count(),12);
}
#[test]
fn v3_real_jit_positive_signed_i64() {
    let f=Fixture::new();
    for (id,expr,expected) in [
        ("near-max-sum","9223372036854775805 + 2","9223372036854775807"),
        ("near-min-successor-two","(-9223372036854775807 - 1) + 2","-9223372036854775806"),
        ("wide-precision-multiply","(9007199254740993 - 9007199254740992) * 41","41"),
        ("signed-precision-three","-9007199254740995 + 9007199254740992","-3"),
        ("trillion-zero-cancel","1000000000000000 + (-1000000000000000)","0"),
        ("mixed-precedence","5 + 7 * 9 - 4 * 3","56"),
        ("right-nested-subtraction-v3","80 - (50 - (25 - 5))","50"),
        ("left-nested-subtraction-v3","((80 - 50) - 25) - 5","0"),
        ("wide-positive-product","2147483647 * 2147483647","4611686014132420609"),
        ("wide-negative-product","(-2147483647) * 2147483647","-4611686014132420609"),
        ("quadruple-negation","-(-(-(-33)))","33"),
        ("max-range-roundtrip","(9223372036854775800 - 9007199254740993) + 9007199254740993","9223372036854775800")
    ] {
        assert!(MANIFEST.contains(&format!("\"expected\": \"{expected}\"")),"manifest expected value drift {id}");
        let out=run_one(&f,id,expr);
        assert!(out.status.success(),"{id} compiler/JIT failure: {}",String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout),format!("{expected}\n"),"{id} wrong guest result");
        assert!(out.stderr.is_empty(),"{id} unexpected guest diagnostics");
    }
}
#[test]
fn v3_real_compiler_rejections_do_not_emit_guest_output() {
    let f=Fixture::new();
    for (id,expr) in [
        ("removed-percent-new","11 % 4"),
        ("removed-caret-new","11 ^ 4"),
        ("removed-double-caret-new","11 ^^ 4"),
        ("positive-outside-signed-range","9223372036854775808 + 1"),
        ("negative-outside-signed-range","-9223372036854775809")
    ] {
        let out=run_one(&f,id,expr);
        assert!(!out.status.success(),"{id} invalid guest source accepted");
        assert!(out.stdout.is_empty(),"{id} rejected guest emitted output");
        assert!(!out.stderr.is_empty(),"{id} lacked compiler diagnostic");
    }
}
#[test]
fn v3_checked_overflow_must_fail_closed_without_guest_success() {
    // Stage-0 JIT cannot yet attest compile rejection vs runtime trap:
    // fail-closed nonzero/no stdout is a weaker check than wasm trap evidence.
    let f=Fixture::new();
    for (id,expr) in [
        ("max-add-two-overflow","9223372036854775807 + 2"),
        ("min-minus-one-overflow","(-9223372036854775807 - 1) - 1"),
        ("positive-square-overflow","3037000500 * 3037000500"),
        ("negative-square-overflow","(-3037000500) * 3037000500")
    ] {
        let out=run_one(&f,id,expr);
        assert!(!out.status.success(),"{id} overflow was accepted");
        assert!(out.stdout.is_empty(),"{id} published wrapped/truncated value");
        assert!(!out.stderr.is_empty(),"{id} lacked overflow/compiler diagnostic");
    }
}
