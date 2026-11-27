//! Native object execution and fail-closed admission for strict fixed arrays.
#![cfg(unix)]

use std::{
    fs,
    io::ErrorKind,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let prefix = format!("atto-strict-fixed-array-{}-{nonce}", std::process::id());
        Self::reserve(&std::env::temp_dir(), &prefix)
    }

    fn reserve(root: &Path, prefix: &str) -> Self {
        // Time resolution can be coarser than concurrent test starts.
        // Claim a private directory atomically; never reuse or remove a
        // preexisting path owned by another test/process.
        for attempt in 0..128 {
            let dir = root.join(format!("{prefix}-{attempt}"));
            match fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => return Self(dir),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot reserve strict-array scratch directory: {e}"),
            }
        }
        panic!("strict-array scratch directory collision budget exhausted")
    }

    fn compile(&self, program: &str) -> Output {
        let source = self.0.join("guest.atto");
        fs::write(&source, program).unwrap();
        Command::new(env!("CARGO_BIN_EXE_orescranelift"))
            .arg("emit-object-strict")
            .arg(source)
            .output()
            .expect("launch real strict Cranelift compiler")
    }

    fn execute_42(&self, artifact: Output, name: &str) {
        assert!(
            artifact.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&artifact.stderr)
        );
        assert!(!artifact.stdout.is_empty(), "{name}: no object emitted");
        let object = self.0.join("guest.o");
        let harness = self.0.join("host.c");
        let binary = self.0.join("guest");
        fs::write(&object, artifact.stdout).unwrap();
        fs::write(
            &harness,
            r#"#include <assert.h>
#include <stdint.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void) {
    int64_t out = -123;
    assert(oreslang_scalar_entry(0,0,0,0,&out) == 0);
    assert(out == 42);
    return 0;
}
"#,
        )
        .unwrap();
        let linked = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
            .arg(&harness)
            .arg(&object)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            linked.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&linked.stderr)
        );
        let guest = Command::new(&binary).output().unwrap();
        assert!(
            guest.status.success(),
            "{name}: linked native guest failed: {:?}",
            guest
        );
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main_program(body: &str) -> String {
    format!("type Bounded = int where self >= 1 and self <= 42; pub fnc main()->int {{ {body} }}")
}

#[test]
fn strict_fixed_array_native_object_values() {
    let dir = Scratch::new();
    for (name, body) in [
        ("singleton", "val cells = Array.of(42); return cells[0];"),
        (
            "literal-index",
            "val cells = Array.of(1,42,3); return cells[1];",
        ),
        (
            "local-index",
            "val cells = Array.of(1,42,3); val index = 1; return cells[index];",
        ),
        (
            "separate-receivers",
            "val first = Array.of(1,40); val second = Array.of(2,99); return first[1]+second[0];",
        ),
        (
            "cell-budget-inclusive",
            "val cells = Array.of(0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,42); return cells[15];",
        ),
    ] {
        dir.execute_42(dir.compile(&main_program(body)), name);
    }
}

#[test]
fn strict_fixed_array_rejections_never_emit_native_bytes() {
    let dir = Scratch::new();
    for (name, body, diagnostic) in [
        (
            "negative-index",
            "val cells = Array.of(42); return cells[-1];",
            "E_VERIFY_UNPROVEN: index not proved within receiver bounds",
        ),
        (
            "upper-bound",
            "val cells = Array.of(42); return cells[1];",
            "E_VERIFY_UNPROVEN: index not proved within receiver bounds",
        ),
        (
            "receiver-escape",
            "val cells = Array.of(42); return cells;",
            "E_VERIFY_UNSUPPORTED: array cannot escape or be used as scalar",
        ),
        (
            "mutable-receiver",
            "let cells: int = Array.of(42); return 42;",
            "E_VERIFY_UNSUPPORTED: fixed array requires inferred val",
        ),
        (
            "explicit-scalar-type",
            "val cells: int = Array.of(42); return 42;",
            "E_VERIFY_UNSUPPORTED: fixed array requires inferred val",
        ),
        (
            "cell-budget-exceeded",
            "val cells = Array.of(0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16); return cells[0];",
            "E_UNIT_LIMIT: fixed array cell budget",
        ),
    ] {
        let result = dir.compile(&main_program(body));
        assert_eq!(result.status.code(), Some(1), "{name}: {:?}", result);
        assert!(result.stdout.is_empty(), "{name}: emitted native bytes");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(diagnostic),
            "{name}: expected {diagnostic}; got {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn scratch_reservation_handles_collisions_without_reusing_foreign_directories() {
    let root = Scratch::new();
    let occupied = root.0.join("collision-0");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("sentinel"), b"untouched").unwrap();
    let children = std::thread::scope(|scope| {
        let root = &root.0;
        let jobs: Vec<_> = (0..16).map(|_| scope.spawn(move || Scratch::reserve(root, "collision"))).collect();
        jobs.into_iter().map(|job| job.join().unwrap()).collect::<Vec<_>>()
    });
    let unique: std::collections::BTreeSet<_> = children.iter().map(|child| &child.0).collect();
    assert_eq!(unique.len(), 16);
    assert!(!unique.contains(&occupied));
    drop(unique);
    drop(children);
    assert_eq!(fs::read(occupied.join("sentinel")).unwrap(), b"untouched");
}
