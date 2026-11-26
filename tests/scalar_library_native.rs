//! Actual source-owned core library object and fail-closed boundary.
#![cfg(target_os = "linux")]
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn real_core_library_compiles_and_links_all_symbols() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("core/types.atto");
    let cli = env!("CARGO_BIN_EXE_orescranelift");
    let ir = Command::new(cli).arg("ir").arg(&source).output().unwrap();
    assert!(
        ir.status.success(),
        "{}",
        String::from_utf8_lossy(&ir.stderr)
    );
    let text = String::from_utf8(ir.stdout).unwrap();
    assert_eq!(text.matches("function ").count(), 13);
    for symbol in ["int_value_bits", "int_min_value", "fits_i8", "fits_u32"] {
        assert!(text.contains(&format!("; {symbol}\n")));
    }
    let obj = Command::new(cli)
        .arg("emit-object")
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        obj.status.success(),
        "{}",
        String::from_utf8_lossy(&obj.stderr)
    );
    assert!(obj.stdout.starts_with(b"\x7fELF"));
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("atto-library-{}-{nonce}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let object = dir.join("core.o");
    let driver = dir.join("probe.c");
    let bin = dir.join("probe");
    fs::write(&object, obj.stdout).unwrap();
    fs::write(
        &driver,
        r#"#include <stdint.h>
#include <limits.h>
extern int64_t int_value_bits(void), bool_value_bits(void), byte_value_bits(void);
extern int64_t int_min_value(void), int_max_value(void);
extern uint8_t fits_i8(int64_t), fits_i16(int64_t), fits_i32(int64_t), fits_i64(int64_t);
extern uint8_t fits_u8(int64_t), fits_u16(int64_t), fits_u32(int64_t), fits_u64(int64_t);
int main(void) {
  if (int_value_bits()!=64 || bool_value_bits()!=1 || byte_value_bits()!=8) return 1;
  if (int_min_value()!=INT64_MIN || int_max_value()!=INT64_MAX) return 2;
  if (!fits_i8(-128)||!fits_i8(127)||fits_i8(-129)||fits_i8(128)) return 3;
  if (!fits_i16(-32768)||fits_i16(32768)||!fits_i32(INT32_MIN)||fits_i32(INT64_MAX)) return 4;
  if (!fits_i64(INT64_MIN)||!fits_i64(INT64_MAX)) return 5;
  if (!fits_u8(255)||fits_u8(-1)||fits_u8(256)||!fits_u16(65535)||fits_u16(65536)) return 6;
  if (!fits_u32(INT64_C(4294967295))||fits_u32(INT64_C(4294967296))) return 7;
  if (!fits_u64(INT64_MAX)||fits_u64(-1)) return 8;
  return 0;
}
"#,
    )
    .unwrap();
    let link = Command::new("cc")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(&driver)
        .arg(&object)
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        link.status.success(),
        "{}",
        String::from_utf8_lossy(&link.stderr)
    );
    assert_eq!(Command::new(&bin).status().unwrap().code(), Some(0));
    // A library is not a synthetic runnable main.
    let run = Command::new(cli).arg("run").arg(&source).output().unwrap();
    assert!(!run.status.success());
    assert!(run.stdout.is_empty());
}

#[test]
fn malformed_or_unproved_library_never_publishes_ir_or_object() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "atto-library-negative-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("bad.atto");
    for body in [
        "pub fnc f()->int{return true;}",
        "pub fnc f()->bool{return 42;}",
        "pub fnc f(int n)->bool{return n && true;}",
        "pub fnc f()->int{return 9223372036854775807+1;}",
        "pub fnc f(int n)->int{return n+1;}",
        "pub fnc f()->int{return missing;}",
        "pub fnc f()->int{return 1;} pub fnc f()->int{return 2;}",
        "pub fnc f(int n, int n)->int{return n;}",
        "pub fnc f()->int{return 1;} trailing",
    ] {
        fs::write(&path, format!("use namespace testlib; {body}")).unwrap();
        for mode in ["check", "ir", "emit-object"] {
            let out = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
                .arg(mode)
                .arg(&path)
                .output()
                .unwrap();
            assert!(!out.status.success(), "{mode}: {body}");
            assert!(out.stdout.is_empty(), "{mode}: {body}");
        }
    }
    fs::remove_dir_all(dir).unwrap();
}
