//! Actual Cranelift CLI + native JIT tests, not a Rust host evaluator.
//! This staged package-link profile uses checked static lowering and does
//! not yet support true native helper-call ABIs / nonlinear parameter use.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const HELPER: &str = "pub fnc increment(int value): int {\n  return value + 1;\n}\n\
    pub fnc twice(int value): int {\n  return value + value;\n}\n\
    fnc private_helper(int value): int {\n  return value + 1;\n}\n";
const ROOT_MAIN: &str = "import increment from \"+src/lib/helpers.atto\";\n\
    pub fnc main(): int { return increment(41); }\n";
const REL_MAIN: &str = "import increment from \"./lib/helpers.atto\";\n\
    pub fnc main(): int { return increment(41); }\n";

struct Project {
    base: PathBuf,
}
impl Project {
    fn new() -> Self {
        for _ in 0..64 {
            let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
            let base = std::env::temp_dir().join(format!(
                "atto-cranelift-link-{}-{ticket}", std::process::id()
            ));
            match fs::create_dir(&base) {
                Ok(()) => {
                    let project = Self { base };
                    project.write(".ores.pkg.toml", "schema_version = \"1\"\n[package]\nname = \"link-native-test\"\n");
                    project.write("src/lib/helpers.atto", HELPER);
                    return project;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create isolated test package: {e}"),
            }
        }
        panic!("test package path exhausted")
    }
    fn write(&self, path: &str, source: &str) -> PathBuf {
        let filename = self.base.join(path);
        fs::create_dir_all(filename.parent().expect("parent")).expect("create local dirs");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&filename)
            .expect("exclusive fixture create");
        file.write_all(source.as_bytes()).expect("write fixture");
        file.sync_all().expect("sync fixture");
        filename
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn invoke(mode: &str, entry: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode)
        .arg(entry)
        .output()
        .expect("execute real Cranelift native compiler")
}
fn reject_all(entry: &Path, expected: &str) {
    for mode in ["check", "ir", "run"] {
        let output = invoke(mode, entry);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{mode} accepted invalid source; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "{mode} emitted CLIF or guest stdout on rejection"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.starts_with("orescranelift: ") && stderr.contains(expected),
            "{mode} wrong diagnostic for {expected}: {stderr}"
        );
        assert!(!stderr.contains("panicked at"), "{mode} host panic: {stderr}");
    }
}

#[test]
fn two_modern_import_spelling_guests_compile_to_real_native_clif_and_run() {
    let project = Project::new();
    for (entry, source) in [
        ("src/main-root.atto", ROOT_MAIN),
        ("src/main-relative.atto", REL_MAIN),
        (
            "src/main-aliased.atto",
            "import increment as add_one from \"+src/lib/helpers.atto\";\n\
             fnc main(): int { return add_one(41); }\n",
        ),
    ] {
        let path = project.write(entry, source);
        let admitted = invoke("check", &path);
        assert_eq!(
            admitted.status.code(),
            Some(0),
            "check {}: {}",
            entry,
            String::from_utf8_lossy(&admitted.stderr)
        );
        assert!(admitted.stdout.is_empty());
        let clif = invoke("ir", &path);
        assert_eq!(
            clif.status.code(),
            Some(0),
            "ir {}: {}",
            entry,
            String::from_utf8_lossy(&clif.stderr)
        );
        assert!(String::from_utf8_lossy(&clif.stdout).contains("function"));
        let native = invoke("run", &path);
        assert_eq!(
            native.status.code(),
            Some(0),
            "run {}: {}",
            entry,
            String::from_utf8_lossy(&native.stderr)
        );
        assert_eq!(native.stdout, b"42\n", "native {entry} result");
        assert!(native.stderr.is_empty());
    }
}

#[test]
fn import_visibility_and_package_authority_fail_before_native_ir() {
    let project = Project::new();
    let cases = [
        (
            "src/reject-private.atto",
            "import private_helper from \"+src/lib/helpers.atto\";\nfnc main(): int { return private_helper(41); }\n",
            "E_LINK_EXPORT_NOT_PUBLIC_OR_MISSING",
        ),
        (
            "src/reject-missing.atto",
            "import missing from \"+src/lib/helpers.atto\";\nfnc main(): int { return missing(41); }\n",
            "E_LINK_EXPORT_NOT_PUBLIC_OR_MISSING",
        ),
        (
            "src/reject-retired-root.atto",
            "import increment from \"+/src/lib/helpers.atto\";\nfnc main(): int { return increment(41); }\n",
            "E_LINK_RETIRED_ROOT_PREFIX",
        ),
        (
            "src/reject-root-traversal.atto",
            "import increment from \"+src/../src/lib/helpers.atto\";\nfnc main(): int { return increment(41); }\n",
            "E_LINK_PATH_TRAVERSAL",
        ),
        (
            "src/reject-duplicate.atto",
            "import increment from \"+src/lib/helpers.atto\";\nimport twice as increment from \"+src/lib/helpers.atto\";\nfnc main(): int { return increment(41); }\n",
            "E_LINK_DUP_BINDING",
        ),
        (
            "src/reject-late-import.atto",
            "fnc main(): int { return increment(41); }\nimport increment from \"+src/lib/helpers.atto\";\n",
            "E_LINK_IMPORT_ORDER",
        ),
        (
            "src/reject-ambient-import.atto",
            "import increment from \"lib/helpers.atto\";\nfnc main(): int { return increment(41); }\n",
            "E_LINK_UNSUPPORTED_IMPORT_KIND",
        ),
    ];
    for (name, source, expected) in cases {
        let entry = project.write(name, source);
        reject_all(&entry, expected);
    }
}

#[test]
fn nonlinear_call_remains_explicitly_unsupported_until_real_native_call_abi() {
    let project = Project::new();
    let path = project.write(
        "src/reject-nonlinear.atto",
        "import twice from \"+src/lib/helpers.atto\";\n\
         fnc main(): int { return twice(21); }\n",
    );
    reject_all(&path, "E_PIPE_NONLINEAR_BINDING_UNSUPPORTED");
}

#[test]
fn graph_imports_may_not_repurpose_isolate_tenant_entry() {
    let project = Project::new();
    let path = project.write("src/isolate-link.atto", ROOT_MAIN);
    let output = invoke("isolate", &path);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("E_LINK_ISOLATE_UNSUPPORTED")
    );
}

#[test]
fn a_nested_manifest_cannot_be_crossed_by_a_project_root_import() {
    let project = Project::new();
    project.write("src/lib/.ores.pkg.toml", "[package]\nname = \"nested\"\n");
    let path = project.write("src/nested.atto", ROOT_MAIN);
    reject_all(&path, "E_LINK_NESTED_PACKAGE");
}

#[cfg(unix)]
#[test]
fn imported_source_symlinks_are_denied_by_descriptor_relative_source_open() {
    use std::os::unix::fs::symlink;
    let project = Project::new();
    let entry = project.write(
        "src/symlink-entry.atto",
        "import escaped from \"./external.atto\";\nfnc main(): int { return escaped(41); }\n",
    );
    let target = project.write(
        "outside.atto",
        "pub fnc escaped(int x): int { return x + 1; }\n",
    );
    symlink(&target, project.base.join("src/external.atto"))
        .expect("create local fixture symlink");
    reject_all(&entry, "E_LINK_SOURCE_OPEN");
}

#[test]
fn oversized_imported_modules_are_rejected_before_any_native_code() {
    let project = Project::new();
    let oversized = format!(
        "pub fnc increment(int x): int {{ return x + 1; }}\n{}",
        " ".repeat(16 * 1024)
    );
    project.write("src/lib/overlarge.atto", &oversized);
    let entry = project.write(
        "src/overlarge.atto",
        "import increment from \"+src/lib/overlarge.atto\";\n\
         fnc main(): int { return increment(41); }\n",
    );
    reject_all(&entry, "E_LINK_SOURCE_BUDGET");
}

#[test]
fn nearest_legacy_package_blocks_parent_modern_authority() {
    let project = Project::new();
    project.write("src/.oreslangc.cfg.toml", "[project]\nname = \"legacy\"\n");
    let entry = project.write("src/legacy-subpackage.atto", ROOT_MAIN);
    reject_all(&entry, "E_LINK_LEGACY_PACKAGE_BOUNDARY");
}

#[test]
fn malformed_modern_manifest_does_not_grant_import_authority() {
    let project = Project::new();
    fs::remove_file(project.base.join(".ores.pkg.toml")).expect("delete test marker");
    project.write(".ores.pkg.toml", "schema_version = \"1\"\n[package]\nname = \"x\"\n[unauthorized]\nsecret = \"path\"\n");
    let entry = project.write("src/malformed-marker.atto", ROOT_MAIN);
    reject_all(&entry, "E_LINK_INVALID_MANIFEST");
}
