//! Exact-byte portable scalar examples. These are real native guest tests when Cargo executes.
//! Never count a zero-step CI run as evidence or infer full Atto actor support.
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Command, Output};

const ORACLES: &str = include_str!("portable-scalar/oracles.tsv");
const FIXTURES: [&str; 6] = [
    "32-grouping-multiplication.ores",
    "33-subtraction-precedence.ores",
    "34-unary-plus-digit-separator.ores",
    "35-upper-bound-difference.ores",
    "36-lower-bound-cancellation.ores",
    "37-named-bitxor.ores",
];

fn run(mode: &str, source: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(source)
        .output()
        .expect("launch the actual native Cranelift compiler")
}

#[test]
fn pinned_portable_scalar_samples_emit_42_and_native_clif() {
    assert!(ORACLES.ends_with('\n') && !ORACLES.contains('\r'));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut seen = HashSet::new();
    let entries: Vec<_> = ORACLES.lines().collect();
    assert_eq!(entries.len(), FIXTURES.len(), "scalar corpus inventory changed");

    for (index, line) in entries.iter().enumerate() {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(), 3, "malformed oracle entry: {line}");
        let name = FIXTURES[index];
        let expected_path = format!("tests/portable-scalar/{name}");
        assert_eq!(fields[0], expected_path, "unexpected fixture or path traversal");
        assert!(seen.insert(fields[0]), "duplicate scalar oracle");
        assert_eq!(fields[1], "42", "the pinned profile should return 42");
        assert_eq!(fields[2].len(), 40, "invalid Git blob digest length");
        assert!(fields[2].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));

        let path = root.join(expected_path);
        let metadata = path.symlink_metadata().expect("pinned scalar fixture missing");
        assert!(metadata.file_type().is_file(), "fixture must be a regular file, not a symlink");
        assert!(metadata.len() > 0 && metadata.len() <= 4096);

        // The corpus records Git blob identity, not SHA-256; use the Git object hash.
        let hash = Command::new("git")
            .arg("hash-object")
            .arg("--")
            .arg(&path)
            .output()
            .expect("git hash-object must be available on the test host");
        assert!(hash.status.success(), "unable to hash {}: {}", path.display(), String::from_utf8_lossy(&hash.stderr));
        assert_eq!(String::from_utf8_lossy(&hash.stdout).trim(), fields[2], "source byte drift: {name}");

        let guest = run("run", &path);
        assert!(guest.status.success(), "guest {name}: {}", String::from_utf8_lossy(&guest.stderr));
        assert!(guest.stderr.is_empty(), "successful guest emitted diagnostics: {name}");
        assert_eq!(guest.stdout, b"42\n", "wrong compiled guest result: {name}");

        let ir = run("ir", &path);
        assert!(ir.status.success(), "IR {name}: {}", String::from_utf8_lossy(&ir.stderr));
        assert!(ir.stderr.is_empty(), "IR emitted diagnostics: {name}");
        assert!(!ir.stdout.is_empty(), "no Cranelift IR emitted for {name}");
        assert!(String::from_utf8_lossy(&ir.stdout).contains("function"), "not CLIF output: {name}");
    }
}
