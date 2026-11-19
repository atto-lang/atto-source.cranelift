//! Negative native admission only, NOT implementation of proposed value/range syntax.
//! An unsupported source rejection cannot satisfy an accept_execute contract case.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const MANIFEST: &str = include_str!("../contracts/value-block-range-v0/cases.json");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Case {
    id: &'static str,
    kind: &'static str,
    source: &'static str,
    json_source: &'static str,
}
const UNSUPPORTED: &[Case] = &[
    Case { id: "block_tail_returns_value", kind: "accept_execute", source: "pub fnc main(): int { const int value = { const int x = 20; x + 22 }; return value; }", json_source: "\"pub fnc main(): int { const int value = { const int x = 20; x + 22 }; return value; }\"" },
    Case { id: "ternary_block_true", kind: "accept_execute", source: "pub fnc main(): int { return true ? { const int x = 40; x + 2 } : { 7 }; }", json_source: "\"pub fnc main(): int { return true ? { const int x = 40; x + 2 } : { 7 }; }\"" },
    Case { id: "ternary_block_false", kind: "accept_execute", source: "pub fnc main(): int { return false ? { 7 } : { const int n = 40; n + 2 }; }", json_source: "\"pub fnc main(): int { return false ? { 7 } : { const int n = 40; n + 2 }; }\"" },
    Case { id: "nested_right_associative", kind: "accept_execute", source: "pub fnc main(): int { return false ? { 1 } : true ? { 42 } : { 3 }; }", json_source: "\"pub fnc main(): int { return false ? { 1 } : true ? { 42 } : { 3 }; }\"" },
    Case { id: "range_exclusive_count", kind: "accept_execute", source: "pub fnc main(): int {\n  let int sum = 0;\n  let int needs_comma = 0;\n  stdio.stdout.write(\"[\");\n  for id in 1..5 {\n    if needs_comma eq 1 then\n      stdio.stdout.write(\",\");\n    fi\n    stdio.stdout.write(id);\n    needs_comma = 1;\n    sum = sum + id;\n  }\n  stdio.stdout.write(\"]\");\n  return sum;\n}", json_source: "\"pub fnc main(): int {\\n  let int sum = 0;\\n  let int needs_comma = 0;\\n  stdio.stdout.write(\\\"[\\\");\\n  for id in 1..5 {\\n    if needs_comma eq 1 then\\n      stdio.stdout.write(\\\",\\\");\\n    fi\\n    stdio.stdout.write(id);\\n    needs_comma = 1;\\n    sum = sum + id;\\n  }\\n  stdio.stdout.write(\\\"]\\\");\\n  return sum;\\n}\"" },
    Case { id: "range_inclusive_count", kind: "accept_execute", source: "pub fnc main(): int {\n  let int sum = 0;\n  let int needs_comma = 0;\n  stdio.stdout.write(\"[\");\n  for id in 1..=5 {\n    if needs_comma eq 1 then\n      stdio.stdout.write(\",\");\n    fi\n    stdio.stdout.write(id);\n    needs_comma = 1;\n    sum = sum + id;\n  }\n  stdio.stdout.write(\"]\");\n  return sum;\n}", json_source: "\"pub fnc main(): int {\\n  let int sum = 0;\\n  let int needs_comma = 0;\\n  stdio.stdout.write(\\\"[\\\");\\n  for id in 1..=5 {\\n    if needs_comma eq 1 then\\n      stdio.stdout.write(\\\",\\\");\\n    fi\\n    stdio.stdout.write(id);\\n    needs_comma = 1;\\n    sum = sum + id;\\n  }\\n  stdio.stdout.write(\\\"]\\\");\\n  return sum;\\n}\"" },
    Case { id: "empty_descending_range", kind: "accept_execute", source: "pub fnc main(): int { let int count = 0; for id in 5..1 { count = count + 1; } return count; }", json_source: "\"pub fnc main(): int { let int count = 0; for id in 5..1 { count = count + 1; } return count; }\"" },
    Case { id: "block_lexical_shadowing", kind: "accept_execute", source: "pub fnc main(): int { const int x = 2; const int y = { const int x = 40; x + 2 }; return y; }", json_source: "\"pub fnc main(): int { const int x = 2; const int y = { const int x = 40; x + 2 }; return y; }\"" },
    Case { id: "unselected_arm_not_evaluated", kind: "accept_execute", source: "pub fnc main(): int { let int n = 0; const int a = true ? { n = n + 1; 42 } : { n = n + 100; 7 }; return a + n; }", json_source: "\"pub fnc main(): int { let int n = 0; const int a = true ? { n = n + 1; 42 } : { n = n + 100; 7 }; return a + n; }\"" },
    Case { id: "empty_collection_context", kind: "accept_typecheck", source: "pub fnc main(): int { const Array<int> xs = true ? { [] } : { [1,2] }; return xs.size(); }", json_source: "\"pub fnc main(): int { const Array<int> xs = true ? { [] } : { [1,2] }; return xs.size(); }\"" },
    Case { id: "block_semicolon_discards_tail", kind: "reject_typecheck", source: "pub fnc main(): int { return true ? { 42; } : { 7 }; }", json_source: "\"pub fnc main(): int { return true ? { 42; } : { 7 }; }\"" },
    Case { id: "block_local_reference_escape", kind: "reject_typecheck", source: "pub fnc main(): ref int { return { const int local = 42; &local }; }", json_source: "\"pub fnc main(): ref int { return { const int local = 42; &local }; }\"" },
    Case { id: "non_bool_ternary", kind: "reject_typecheck", source: "pub fnc main(): int { return 1 ? { 42 } : { 7 }; }", json_source: "\"pub fnc main(): int { return 1 ? { 42 } : { 7 }; }\"" },
    Case { id: "range_chaining", kind: "reject_parse", source: "pub fnc main(): int { for id in 1..3..5 { } return 0; }", json_source: "\"pub fnc main(): int { for id in 1..3..5 { } return 0; }\"" },
    Case { id: "inclusive_max_endpoint", kind: "accept_execute", source: "pub fnc main(): int { let int count = 0; for n in 9223372036854775807..=9223372036854775807 { count = count + 1; } return count; }", json_source: "\"pub fnc main(): int { let int count = 0; for n in 9223372036854775807..=9223372036854775807 { count = count + 1; } return count; }\"" },
    Case { id: "range_bounds_eval_once", kind: "accept_execute", source: "pub fnc main(): int { let int x = 0; const int low = { x = x + 1; 1 }; const int high = { x = x + 1; 3 }; for id in low..=high { } return x; }", json_source: "\"pub fnc main(): int { let int x = 0; const int low = { x = x + 1; 1 }; const int high = { x = x + 1; 3 }; for id in low..=high { } return x; }\"" },
];

struct Source(PathBuf);
impl Source {
    fn new(source: &str) -> Self {
        assert!(!source.is_empty() && source.len() <= 1024 * 1024);
        for _ in 0..64 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "atto-cranelift-value-range-{}-{id}.atto", std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(source.as_bytes()).expect("write source fixture");
                    file.sync_all().expect("sync source fixture");
                    return Self(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("unable to create exclusive source: {error}"),
            }
        }
        panic!("unable to reserve source fixture after 64 attempts");
    }
}
impl Drop for Source {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}
fn invoke(mode: &str, source: &Source) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg(mode).arg(&source.0).output().expect("invoke actual native compiler")
}

fn check_pinned_contract(case: &Case) {
    let marker = format!("\"id\": \"{}\"", case.id);
    assert_eq!(MANIFEST.matches(marker.as_str()).count(), 1, "missing or duplicated contract id");
    let start = MANIFEST.find(marker.as_str()).expect("case ID must exist");
    let slice = &MANIFEST[start..];
    let end = slice.find("\n    },").expect("missing JSON case terminator");
    let record = &slice[..end];
    assert!(record.contains(&format!("\"kind\": \"{}\"", case.kind)), "kind drift: {}", case.id);
    assert!(record.contains(&format!("\"source\": {}", case.json_source)), "source drift: {}", case.id);
}

#[test]
fn scalar_positive_control_and_proposed_blocks_ranges_reject_before_jit() {
    assert!(MANIFEST.contains("\"schema\": \"attolang.value-block-range.v0\""));
    assert!(MANIFEST.contains("\"status\": \"PROPOSED_NOT_COMPILER_VERIFIED\""));
    assert!(MANIFEST.contains("\"requires_compiled_guest_for_pass\": true"));
    assert_eq!(UNSUPPORTED.len(), 16, "all 16 standalone contract programs must be checked");

    // The negative suite is meaningless without a *real* positive compiled guest.
    let positive = Source::new("fnc main():int{return 40+2;}\n");
    let run = invoke("run", &positive);
    assert_eq!(run.status.code(), Some(0), "scalar guest: {}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(run.stdout, b"42\n", "positive control returned the wrong value");
    assert!(run.stderr.is_empty());
    let ir = invoke("ir", &positive);
    assert_eq!(ir.status.code(), Some(0), "scalar IR: {}", String::from_utf8_lossy(&ir.stderr));
    assert!(!ir.stdout.is_empty());
    assert!(ir.stderr.is_empty());

    // Stage-0 is scalar-only. Future accept_execute and accept_typecheck cases
    // are *not* counted as passing; instead, test explicit unsupported denial.
    for case in UNSUPPORTED {
        check_pinned_contract(case);
        let guest = Source::new(case.source);
        for mode in ["run", "ir"] {
            let result = invoke(mode, &guest);
            assert_eq!(result.status.code(), Some(1), "{} unexpectedly admitted {}", mode, case.id);
            assert!(result.stdout.is_empty(), "{} published unsupported native output for {}", mode, case.id);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(diagnostic.starts_with("orescranelift: "), "{} lacks compiler rejection: {}", case.id, diagnostic);
            assert!(!diagnostic.contains("panicked at"), "{} panicked rather than rejecting: {}", case.id, diagnostic);
        }
    }
}
