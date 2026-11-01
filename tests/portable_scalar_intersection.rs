//! Shared source-identical portable-scalar corpus: compiler -> native JIT.
//! These are not host arithmetic or verified cross-backend result claims.
use std::path::Path;
use std::process::Command;

#[test]
fn identical_scalar_guest_bytes_execute_to_42() {
    // Git blob SHA-1 IDs of programs 34..49 from the reviewed parity corpus
    // at oreslang-compilers-parity-testing PR #16. Do not rewrite the sources
    // per backend: the hash is an exact-byte drift gate.
    let fixtures = [
        ("34-grouped-precedence-42.ores", "63e6a7d1781aa11241d992eb3f46ea094741ed40"),
        ("35-double-negation-42.ores", "b286c272504e8bee5ff2f4cfc058f605c821d3d6"),
        ("36-left-associative-subtraction-42.ores", "b679c52fda15b1416cbc4101f57d2e92cccb4233"),
        ("37-signed-multiplication-42.ores", "75998eef56fff1fa6861bd03f3d2501101aa51b6"),
        ("38-negative-multiplication-chain-42.ores", "b18646e5d75b6324a53683dfc0dd02d0749a32fb"),
        ("39-maximum-minus-large-42.ores", "933170e4f02620ec5201c65e6cdfc7ef52c801c2"),
        ("40-min-plus-max-42.ores", "d0be662540d37a0caf21db846c10913ce09d7d65"),
        ("41-nested-grouping-42.ores", "1963f09603b5e78da27e6e26382f47b3cc132d92"),
        ("42-immutable-default-local.ores", "bef40c26e62968279635ca4f0b3422aa0b500beb"),
        ("43-const-typed-local.ores", "328ab99f1ad6051090c09f302e38a5389c7362ae"),
        ("44-annotated-const-int.ores", "616abb5c2bf0109af91377d0e6da76f41bf9c1eb"),
        ("45-cast-int-literal.ores", "69cecd231d32bf70ea36424e59c932861d01fe44"),
        ("46-chained-cast-int.ores", "b088dd3f5e3114be4affbb4935582b58f010d75f"),
        ("47-grouped-cast-multiply.ores", "9ebc3f7bc230557fe2f2639172883bb2f4589464"),
        ("48-readonly-local-cast.ores", "87d0773504e0d22fc3cf1ee76755b36a2fc415c6"),
        ("49-const-typed-identity.ores", "bff7c6070008c5c6a8b3316e8a289f93d3f8c5b3"),
    ];
    for (name, expected_git_blob) in fixtures {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("parity")
            .join(name);
        let metadata = std::fs::symlink_metadata(&path).expect("missing pinned guest fixture");
        assert!(metadata.file_type().is_file(), "not a regular file: {name}");
        assert!(metadata.len() > 0 && metadata.len() <= 4096, "invalid fixture size: {name}");
        let hash = Command::new("git")
            .arg("hash-object")
            .arg("--")
            .arg(&path)
            .output()
            .expect("git is required for immutable corpus-blob verification");
        assert!(
            hash.status.success(),
            "unable to fingerprint guest source {name}: {}",
            String::from_utf8_lossy(&hash.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&hash.stdout).trim(),
            expected_git_blob,
            "source drift detected: {name}"
        );

        let result = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("run")
            .arg(&path)
            .output()
            .expect("native Cranelift CLI failed to start");
        assert!(
            result.status.success(),
            "native guest {name} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            result.stdout,
            b"42\n",
            "different native guest result for {name}"
        );
    }
}
