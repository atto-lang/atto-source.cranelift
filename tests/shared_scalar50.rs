//! Exact shared scalar50 source bytes, executed via the real Cranelift JIT.
//! .atto is used for new runtime test scratch files; the pinned TSV is unchanged.
use std::collections::HashSet;
#[path = "support/mod.rs"]
mod support;

const CORPUS: &str = include_str!("../contracts/parity/scalar50/v1.tsv");

#[test]
fn scalar50_native_jit_exact_shared_sources() {
    assert!(CORPUS.ends_with('\n') && !CORPUS.contains('\r'));
    let mut ids = HashSet::new();
    let mut count = 0_usize;
    for row in CORPUS.lines().filter(|line| !line.starts_with('#') && !line.is_empty()) {
        let fields: Vec<_> = row.split('\t').collect();
        assert_eq!(fields.len(), 3, "invalid source corpus record");
        count += 1;
        assert_eq!(fields[0], format!("scalar-{count:03}"));
        assert!(ids.insert(fields[0]), "duplicate scalar case");
        let expected: i64 = fields[1].parse().expect("canonical positive i64 expectation");
        assert!((1..128).contains(&expected));
        let source = fields[2];
        assert!(source.starts_with("pub fnc main(): int { return "));
        assert!(source.ends_with("; }"));
        assert!(source.len() < 256);
        let output = support::run_source("run", source.as_bytes(), &[]);
        assert!(
            output.status.success(),
            "{}: native JIT failed: {}",
            fields[0],
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{}: unexpected stderr", fields[0]);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{expected}\n"),
            "{}: native JIT result drift",
            fields[0]
        );
    }
    assert_eq!(count, 50, "scalar50 inventory drift");
}
