#![cfg(unix)]
//! Actual Cranelift CLI + native JIT tests, not a Rust host evaluator.
//! This restricted trusted scalar profile emits independent native
//! Cranelift helper calls; it is not a sandbox, actor backend or AOT proof.
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
                "atto-cranelift-link-{}-{ticket}",
                std::process::id()
            ));
            match fs::create_dir(&base) {
                Ok(()) => {
                    let project = Self { base };
                    project.write(
                        ".ores.pkg.toml",
                        "schema_version = \"1\"\n[package]\nname = \"link-native-test\"\n",
                    );
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
        assert!(
            !stderr.contains("panicked at"),
            "{mode} host panic: {stderr}"
        );
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
fn imported_nonlinear_helper_uses_real_native_call_and_returns_42() {
    let project = Project::new();
    let path = project.write(
        "src/native-double.atto",
        "import twice from \"+src/lib/helpers.atto\";\n\
         pub fnc main(): int { return twice(21); }\n",
    );
    let check = invoke("check", &path);
    assert_eq!(
        check.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.stdout.is_empty());
    let ir = invoke("ir", &path);
    assert_eq!(
        ir.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(
        String::from_utf8_lossy(&ir.stdout).contains("call "),
        "no native CLIF call"
    );
    let run = invoke("run", &path);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"42\n");
    assert!(run.stderr.is_empty());
}

#[test]
fn nested_public_calls_and_lexical_val_initializers_execute_as_native_functions() {
    let project = Project::new();
    let path = project.write(
        "src/nested-native.atto",
        "import increment from \"+src/lib/helpers.atto\";\n\
         import twice from \"+src/lib/helpers.atto\";\n\
         pub fnc main(): int {\n\
           val intermediate = increment(20);\n\
           return twice(intermediate);\n\
         }\n",
    );
    let check = invoke("check", &path);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let ir = invoke("ir", &path);
    assert!(
        ir.status.success(),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    let source = String::from_utf8_lossy(&ir.stdout);
    assert!(
        source.matches("call ").count() >= 2,
        "nested native calls missing"
    );
    let run = invoke("run", &path);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"42\n");
}

#[test]
fn checked_overflow_in_a_callee_or_an_argument_propagates_before_entry_output() {
    let project = Project::new();
    for (name, source) in [
        (
            "src/callee-overflow.atto",
            "import twice from \"+src/lib/helpers.atto\";\n\
             pub fnc main(): int { return twice(9223372036854775807); }\n",
        ),
        (
            "src/argument-overflow.atto",
            "import twice from \"+src/lib/helpers.atto\";\n\
             pub fnc main(): int { return twice(9223372036854775807 + 1); }\n",
        ),
    ] {
        let path = project.write(name, source);
        assert!(
            invoke("check", &path).status.success(),
            "{name} source rejected"
        );
        assert!(invoke("ir", &path).status.success(), "{name} CLIF rejected");
        let observed = invoke("run", &path);
        assert_eq!(observed.status.code(), Some(1), "{name} overflow wrapped");
        assert!(
            observed.stdout.is_empty(),
            "{name} wrote an output despite failure"
        );
        assert!(
            String::from_utf8_lossy(&observed.stderr).contains("i64 arithmetic overflow"),
            "{name}: {}",
            String::from_utf8_lossy(&observed.stderr)
        );
    }
}

#[test]
fn guest_linked_calls_reject_bad_arity_missing_symbol_and_recursive_graph() {
    let project = Project::new();
    let wrong_arity = project.write(
        "src/invalid-arity.atto",
        "import twice from \"+src/lib/helpers.atto\";\n\
         pub fnc main(): int { return twice(10, 11); }\n",
    );
    reject_all(&wrong_arity, "E_LINK_CALL_ARITY_OR_ENTRY");
    let unbound = project.write(
        "src/lib/unbound.atto",
        "pub fnc wrong(int x): int { return unknown(x); }\n",
    );
    assert!(unbound.is_file());
    let unbound_entry = project.write(
        "src/invalid-unbound.atto",
        "import wrong from \"+src/lib/unbound.atto\";\n\
         pub fnc main(): int { return 42; }\n",
    );
    reject_all(&unbound_entry, "E_LINK_UNBOUND_CALLEE");
    let cyclic = project.write(
        "src/lib/cycle.atto",
        "pub fnc cycle(int x): int { return cycle(x); }\n",
    );
    assert!(cyclic.is_file());
    let cyclic_entry = project.write(
        "src/invalid-cycle.atto",
        "import cycle from \"+src/lib/cycle.atto\";\n\
         pub fnc main(): int { return 42; }\n",
    );
    reject_all(&cyclic_entry, "E_LINK_CALL_CYCLE");
}

#[test]
fn modern_linked_imports_do_not_reinterpret_legacy_ores_entries() {
    let project = Project::new();
    let path = project.write(
        "src/legacy-linked.ores",
        "import twice from \"+src/lib/helpers.atto\";\n\
         pub fnc main(): int { return twice(21); }\n",
    );
    reject_all(&path, "E_LINK_ENTRY_REQUIRES_ATTO");
}

#[test]
fn graph_imports_may_not_repurpose_isolate_tenant_entry() {
    let project = Project::new();
    let path = project.write("src/isolate-link.atto", ROOT_MAIN);
    let output = invoke("isolate", &path);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("E_LINK_ISOLATE_UNSUPPORTED"));
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
    symlink(&target, project.base.join("src/external.atto")).expect("create local fixture symlink");
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
    project.write(
        ".ores.pkg.toml",
        "schema_version = \"1\"\n[package]\nname = \"x\"\n[unauthorized]\nsecret = \"path\"\n",
    );
    let entry = project.write("src/malformed-marker.atto", ROOT_MAIN);
    reject_all(&entry, "E_LINK_INVALID_MANIFEST");
}

#[test]
fn discarded_parameter_still_eagerly_evaluates_and_traps() {
    let project = Project::new();
    project.write(
        "src/lib/select_first.atto",
        "pub fnc select_first(int a, int b): int { return a; }\n",
    );
    let source = project.write(
        "src/eager-discarded-argument.atto",
        "import select_first from \"+src/lib/select_first.atto\";\n\
         pub fnc main(): int {\n\
           return select_first(42, 9223372036854775807 + 1);\n\
         }\n",
    );
    assert!(invoke("check", &source).status.success());
    assert!(invoke("ir", &source).status.success());
    let run = invoke("run", &source);
    assert_eq!(run.status.code(), Some(1));
    assert!(
        run.stdout.is_empty(),
        "discarded eager argument must still trap"
    );
    assert!(String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"));
}

#[test]
fn unused_val_initializer_still_executes_and_traps_before_return() {
    let project = Project::new();
    project.write(
        "src/lib/eager.atto",
        "pub fnc eager(int x): int {\n\
           val unused = x + 1;\n\
           return 42;\n\
         }\n",
    );
    let path = project.write(
        "src/unused-val-error.atto",
        "import eager from \"+src/lib/eager.atto\";\n\
         pub fnc main(): int { return eager(9223372036854775807); }\n",
    );
    assert!(invoke("check", &path).status.success());
    assert!(invoke("ir", &path).status.success());
    let run = invoke("run", &path);
    assert_eq!(run.status.code(), Some(1));
    assert!(run.stdout.is_empty());
    assert!(String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"));
}

#[test]
fn standalone_multi_function_atto_uses_real_native_calls_without_a_manifest() {
    let project = Project::new();
    fs::remove_file(project.base.join(".ores.pkg.toml")).expect("remove unused marker");
    let entry = project.write(
        "src/standalone-native.atto",
        r#"pub fnc twice(int value): int {
  return value + value;
}
pub fnc main(): int {
  return twice(21);
}
"#,
    );
    let check = invoke("check", &entry);
    assert_eq!(
        check.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.stdout.is_empty());
    let ir = invoke("ir", &entry);
    assert_eq!(
        ir.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    let clif = String::from_utf8_lossy(&ir.stdout);
    assert!(
        clif.contains("call "),
        "standalone helper was not a native call"
    );
    assert!(
        clif.contains("linked function twice"),
        "missing helper CLIF"
    );
    assert!(clif.contains("linked function main"), "missing entry CLIF");
    let native = invoke("run", &entry);
    assert_eq!(
        native.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(native.stdout, b"42\n");
    assert!(native.stderr.is_empty());
}

#[test]
fn standalone_multi_function_dormant_errors_fail_before_clif_or_execution() {
    let project = Project::new();
    for (name, source, expected) in [
        (
            "src/standalone-unbound.atto",
            "fnc unused(int x): int { return missing(x); }\nfnc main(): int { return 42; }\n",
            "E_LINK_UNBOUND_CALLEE",
        ),
        (
            "src/standalone-cycle.atto",
            "fnc unused(int x): int { return unused(x); }\nfnc main(): int { return 42; }\n",
            "E_LINK_CALL_CYCLE",
        ),
        (
            "src/standalone-duplicate.atto",
            "fnc main(): int { return 0; }\nfnc main(): int { return 42; }\n",
            "E_LINK_DUP_FUNCTION",
        ),
    ] {
        reject_all(&project.write(name, source), expected);
    }
}

#[test]
fn standalone_checked_helper_overflow_never_writes_guest_output() {
    let project = Project::new();
    let entry = project.write(
        "src/standalone-checked.atto",
        r#"pub fnc twice(int x): int { return x + x; }
pub fnc main(): int { return twice(9223372036854775807); }
"#,
    );
    assert!(invoke("check", &entry).status.success());
    assert!(invoke("ir", &entry).status.success());
    let run = invoke("run", &entry);
    assert_eq!(run.status.code(), Some(1));
    assert!(run.stdout.is_empty());
    assert!(String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"));
}

#[test]
fn standalone_name_first_formals_keep_real_native_calls_and_checked_status() {
    let project = Project::new();
    let entry = project.write(
        "src/name-first-double.atto",
        "pub fnc twice(value: int): int { return value + value; }\n         pub fnc main(): int { return twice(21); }\n",
    );
    assert!(invoke("check", &entry).status.success());
    let ir = invoke("ir", &entry);
    assert!(
        ir.status.success(),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    assert!(String::from_utf8_lossy(&ir.stdout).contains("call "));
    let run = invoke("run", &entry);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"42\n");
}

#[test]
fn standalone_name_first_and_type_first_formals_are_equivalent_but_untyped_rejects() {
    let project = Project::new();
    for (name, callable) in [
        (
            "typed-first",
            "fnc twice(int value): int { return value + value; }",
        ),
        (
            "named-first",
            "fnc twice(value: int): int { return value + value; }",
        ),
    ] {
        let entry = project.write(
            &format!("src/{name}.atto"),
            &format!("{callable}\nfnc main(): int {{ return twice(21); }}\n"),
        );
        assert_eq!(invoke("run", &entry).stdout, b"42\n", "{name}");
    }
    for (name, bad_formal) in [
        ("untyped", "value"),
        ("wrong-type", "value: bool"),
        ("duplicate", "int value, value: int"),
    ] {
        let entry = project.write(
            &format!("src/deny-{name}.atto"),
            &format!("fnc twice({bad_formal}): int {{ return 42; }}\n                fnc main(): int {{ return twice(21); }}\n"),
        );
        for mode in ["check", "ir", "run"] {
            let result = invoke(mode, &entry);
            assert_eq!(result.status.code(), Some(1), "{name} {mode} admitted");
            assert!(result.stdout.is_empty(), "{name} {mode} emitted output");
        }
    }
}

#[test]
fn standalone_inlineable_calls_use_real_native_function_abi_not_expression_substitution() {
    let project = Project::new();
    fs::remove_file(project.base.join(".ores.pkg.toml"))
        .expect("standalone compilation must not need a package marker");
    let entry = project.write(
        "src/standalone-linear-calls.atto",
        "fnc double(int x): int { return x * 2; }\n\
         fnc add_two(int x): int { return x + 2; }\n\
         pub fnc main(): int { return add_two(double(20)); }\n",
    );
    let check = invoke("check", &entry);
    assert_eq!(
        check.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.stdout.is_empty(), "check must be non-executing");
    let ir = invoke("ir", &entry);
    assert_eq!(
        ir.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    let clif = String::from_utf8_lossy(&ir.stdout);
    assert!(
        clif.contains("; linked function double"),
        "standalone double has no CLIF"
    );
    assert!(
        clif.contains("; linked function add_two"),
        "standalone add_two has no CLIF"
    );
    assert!(
        clif.contains("; linked function main"),
        "standalone main has no CLIF"
    );
    assert!(
        clif.matches("call ").count() >= 2,
        "nested helpers were inlined: {clif}"
    );
    let run = invoke("run", &entry);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"42\n");
    assert!(run.stderr.is_empty());
}

#[test]
fn standalone_native_calls_preserve_checked_eager_arguments_and_unused_val_initializers() {
    let project = Project::new();
    let cases = [
        (
            "src/standalone-discarded-argument.atto",
            "fnc first(int a, int b): int { return a; }\n\
             pub fnc main(): int { return first(42, 9223372036854775807 + 1); }\n",
        ),
        (
            "src/standalone-unused-val.atto",
            "fnc first(int x): int { val unused = x + 1; return 42; }\n\
             pub fnc main(): int { return first(9223372036854775807); }\n",
        ),
    ];
    for (name, source) in cases {
        let entry = project.write(name, source);
        let check = invoke("check", &entry);
        assert!(
            check.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        let ir = invoke("ir", &entry);
        assert!(
            ir.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&ir.stderr)
        );
        assert!(
            String::from_utf8_lossy(&ir.stdout).contains("call "),
            "{name}: missing native helper call"
        );
        let run = invoke("run", &entry);
        assert_eq!(run.status.code(), Some(1), "{name} incorrectly succeeded");
        assert!(
            run.stdout.is_empty(),
            "{name} emitted a successful result on overflow"
        );
        assert!(
            String::from_utf8_lossy(&run.stderr).contains("i64 arithmetic overflow"),
            "{name}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn multiline_block_comment_with_import_looking_text_is_not_a_module_import() {
    let project = Project::new();
    fs::remove_file(project.base.join(".ores.pkg.toml"))
        .expect("comment-only source needs no package marker");
    let entry = project.write(
        "src/multiline-comment.atto",
        "/* narrative only\n\
         import missing from \"+src/private/absent.atto\";\n\
         /* nested import ignored too */\n\
         */\n\
         pub fnc main(): int { return 42; }\n",
    );
    for mode in ["check", "ir", "run"] {
        let outcome = invoke(mode, &entry);
        assert_eq!(
            outcome.status.code(),
            Some(0),
            "{mode}: {}",
            String::from_utf8_lossy(&outcome.stderr)
        );
        if mode == "run" {
            assert_eq!(outcome.stdout, b"42\n");
        } else if mode == "check" {
            assert!(outcome.stdout.is_empty(), "check may not emit guest output");
        }
    }
}

#[test]
fn genuine_import_after_unsupported_block_comment_still_fails_closed() {
    let project = Project::new();
    let entry = project.write(
        "src/block-comment-plus-import.atto",
        "/* deliberately unsupported module-header comment */\n\
         import twice from \"+src/lib/helpers.atto\";\n\
         pub fnc main(): int { return twice(21); }\n",
    );
    for mode in ["check", "ir", "run"] {
        let outcome = invoke(mode, &entry);
        assert_eq!(
            outcome.status.code(),
            Some(1),
            "{mode} admitted import with block comment"
        );
        assert!(
            outcome.stdout.is_empty(),
            "{mode} emitted output from rejected source"
        );
    }
}

#[test]
fn shared_atto_xo_debit_corpus_executes_unchanged_linked_native_guests() {
    const CORPUS: &str = include_str!("../contracts/atto-xo-checked-debit-v1/cases.tsv");
    let corpus_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contracts/atto-xo-checked-debit-v1/cases.tsv");
    let digest = Command::new("git")
        .arg("hash-object")
        .arg(&corpus_path)
        .output()
        .expect("hash exact shared source corpus");
    assert!(digest.status.success());
    assert_eq!(
        String::from_utf8(digest.stdout).unwrap().trim(),
        "6831dbd0c18eeaa2078fa5f3e324d0f88b40771e"
    );
    let project = Project::new();
    fs::remove_file(project.base.join(".ores.pkg.toml")).expect("remove unused marker");
    let mut seen = std::collections::HashSet::new();
    let mut values = 0;
    let mut overflows = 0;
    for line in CORPUS.lines() {
        let fields: Vec<_> = line.split('|').collect();
        let [id, expected, _, _, _, _, _, source]: [&str; 8] =
            fields.try_into().expect("eight reviewed corpus columns");
        assert!(seen.insert(id), "duplicate case {id}");
        let entry = project.write(&format!("src/debit-{id}.atto"), &format!("{source}\n"));
        let check = invoke("check", &entry);
        assert_eq!(
            (check.status.code(), check.stdout.as_slice()),
            (Some(0), b"".as_slice()),
            "{id} admission: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        assert!(check.stderr.is_empty());
        let ir = invoke("ir", &entry);
        assert_eq!(ir.status.code(), Some(0), "{id} IR");
        assert!(ir.stderr.is_empty());
        let clif = String::from_utf8(ir.stdout).expect("CLIF UTF-8");
        assert!(clif.contains("linked function debit"), "{id} missing helper");
        assert!(clif.contains("linked function main"), "{id} missing entry");
        assert!(clif.contains("call "), "{id} missing native call");

        let run = invoke("run", &entry);
        if expected == "overflow" {
            assert_eq!(run.status.code(), Some(1), "{id} overflow wrapped");
            assert!(run.stdout.is_empty(), "{id} emitted a partial value");
            let diagnostic = String::from_utf8_lossy(&run.stderr);
            assert!(
                diagnostic.starts_with("orescranelift: ")
                    && diagnostic.contains("i64 arithmetic overflow")
                    && !diagnostic.contains("panicked at"),
                "{id} unexpected failure: {diagnostic}"
            );
            overflows += 1;
        } else {
            assert!(matches!(expected, "42" | "43"), "{id} invalid expected value");
            assert_eq!(
                run.status.code(),
                Some(0),
                "{id}: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(run.stdout, format!("{expected}\n").as_bytes(), "{id}");
            assert!(run.stderr.is_empty());
            values += 1;
        }
    }
    assert_eq!((seen.len(), values, overflows), (7, 3, 4));
}
