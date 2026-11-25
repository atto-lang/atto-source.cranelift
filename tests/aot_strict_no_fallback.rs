//! Source CLI proof for linked scalar AOT objects and strict AOT admission.
use std::process::Command;

#[test]
fn unsupported_aot_modes_fail_before_source_access_or_jit() {
    let bin = env!("CARGO_BIN_EXE_orescranelift");
    for mode in ["aot", "full-aot"] {
        let out = Command::new(bin)
            .arg(mode)
            .arg("this-source-must-not-be-opened.atto")
            .arg("this-output-must-not-be-written.o")
            .output()
            .expect("launch actual native Cranelift CLI");
        assert_eq!(out.status.code(), Some(1), "{mode} must be rejected");
        assert!(
            out.stdout.is_empty(),
            "{mode} must not print a guest result"
        );
        let diagnostic = String::from_utf8_lossy(&out.stderr);
        assert!(
            diagnostic.starts_with("orescranelift: ")
                && diagnostic.contains("full AOT executable linking is not implemented")
                && diagnostic.contains("refusing JIT fallback"),
            "{mode} must return a compiler-owned AOT denial: {diagnostic}"
        );
    }
}

#[test]
fn object_mode_is_not_misrepresented_as_supported_aot() {
    let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("object")
        .arg("this-source-must-not-be-opened.atto")
        .arg("this-output-must-not-be-written.o")
        .output()
        .expect("launch native CLI for unsupported object mode");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage: orescranelift"),
        "unknown object command must fail closed"
    );
}

#[cfg(unix)]
#[test]
fn linked_scoped_object_preserves_result_and_closes_on_checked_failure() {
    let directory = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("atto-cranelift-scope-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let input = directory.join("scope.atto");
    std::fs::write(
        &input,
        r#"
        pub fnc main(int input): int {
            using telemetry.span("outer") as outer {
                using telemetry.span("inner") as inner { return input + 1; }
            }
            return 0;
        }
    "#,
    )
    .unwrap();
    let artifact = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("emit-object")
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        artifact.status.success(),
        "{}",
        String::from_utf8_lossy(&artifact.stderr)
    );
    let object = directory.join("guest.o");
    std::fs::write(&object, artifact.stdout).unwrap();
    let harness = directory.join("harness.c");
    std::fs::write(
        &harness,
        r#"
        #include <assert.h>
        #include <stdint.h>
        #include <string.h>

        extern int32_t oreslang_scalar_entry(int64_t, int64_t, int64_t, int64_t, int64_t*);
        static unsigned enabled, started, ended, depth;
        static uint64_t active[2];
        uint64_t ores_native_telemetry_span_begin(const char* name, uint64_t length) {
            if (!enabled) return 0;
            assert(depth < 2 && length == 5);
            assert(memcmp(name, depth ? "inner" : "outer", length) == 0);
            uint64_t id = ++started;
            active[depth++] = id;
            return id;
        }
        void ores_native_telemetry_span_end(uint64_t token) {
            if (token == 0) return;
            assert(depth && active[--depth] == token);
            ++ended;
        }
        int main(void) {
            int64_t result = 0;
            assert(oreslang_scalar_entry(41, 0, 0, 0, &result) == 0 && result == 42);
            assert(!started && !ended && !depth);
            enabled = 1;
            assert(oreslang_scalar_entry(41, 0, 0, 0, &result) == 0 && result == 42);
            assert(started == 2 && ended == 2 && !depth);
            assert(oreslang_scalar_entry(INT64_MAX, 0, 0, 0, &result) == 1 && result == 42);
            assert(started == 4 && ended == 4 && !depth);
        }
    "#,
    )
    .unwrap();
    let binary = directory.join("guest");
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
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    assert!(Command::new(&binary).status().unwrap().success());
    std::fs::remove_dir_all(directory).unwrap();
}
