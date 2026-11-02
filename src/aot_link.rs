#[cfg(unix)]
mod native {
//! AOT native link step for a deliberately narrow scalar guest entry.
//! The compiler supplies the fixed, source-independent entrypoint; the guest
//! is already lowered into machine code in the embedded Cranelift object.
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs;
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::Command;

struct PrivateDirectory(PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Build a private temporary directory on the destination filesystem,
/// with atomic, no-overwrite final publication by hard link.
fn create_private_directory(output: &Path) -> Result<PrivateDirectory, String> {
    let parent = output.parent().filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err("AOT output parent must be an existing directory".into());
    }
    let pattern = parent.join(".atto-aot-build-XXXXXXXX");
    let mut bytes = CString::new(pattern.as_os_str().as_bytes())
        .map_err(|_| "AOT output path contains NUL")?
        .into_bytes_with_nul();
    // SAFETY: mkdtemp requires an owned, writable, NUL-terminated template
    // ending in six Xs. It atomically creates a 0700 private directory.
    let ptr = unsafe { libc::mkdtemp(bytes.as_mut_ptr().cast()) };
    if ptr.is_null() {
        return Err(format!("cannot create private AOT build directory: {}",
                           std::io::Error::last_os_error()));
    }
    // SAFETY: mkdtemp returns the same mutable buffer, still NUL terminated.
    let path = unsafe { CStr::from_ptr(ptr) };
    Ok(PrivateDirectory(PathBuf::from(OsStr::from_bytes(path.to_bytes()))))
}

fn write_private_file(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new().write(true).create_new(true)
        .open(path).map_err(|e| format!("cannot write AOT staging file: {e}"))?;
    file.write_all(data).and_then(|_| file.sync_all())
        .map_err(|e| format!("cannot sync AOT staging file: {e}"))
}

/// Link the program-specific Cranelift ELF/Mach-O object with a fixed
/// compiler-provided native entrypoint. rustc and the system linker are
/// *build-time* dependencies; the installed binary contains neither a
/// source compiler nor a macro evaluator.
pub fn link_executable(object: &[u8], arity: usize, output: &Path) -> Result<(), String> {
    if arity > 4 {
        return Err("AOT entrypoint cannot accept more than four scalar arguments".into());
    }
    if output.file_name().is_none() {
        return Err("AOT output must be an executable file path".into());
    }
    let private = create_private_directory(output)?;
    let guest_object = private.0.join("guest.o");
    let source = private.0.join("aot_entry.rs");
    let binary = private.0.join("guest-executable");
    write_private_file(&guest_object, object)?;
    write_private_file(&source, include_str!("aot_entry.rs").as_bytes())?;

    let mut linkarg = OsString::from("link-arg=");
    linkarg.push(&guest_object);
    let result = Command::new("rustc")
        .arg("--edition=2024")
        .args(["-C", "opt-level=2", "-C", "panic=abort", "-C"])
        .arg(linkarg)
        .arg("--")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .env("ATTO_AOT_ARITY", arity.to_string())
        .output()
        .map_err(|e| format!("cannot invoke build-time native linker: {e}"))?;
    if !result.status.success() {
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        return Err(format!("AOT native linker failed: {}", diagnostic.chars().take(8192).collect::<String>()));
    }
    if !binary.metadata().map_err(|e| format!("missing linked executable: {e}"))?.is_file() {
        return Err("native linker did not emit a regular executable".into());
    }
    // Publish a complete executable in one no-clobber operation. A staging
    // failure or an existing destination leaves the destination unchanged.
    fs::hard_link(&binary, output)
        .map_err(|e| format!("AOT destination exists or publication failed: {e}"))?;
    Ok(())
}

}
#[cfg(unix)]
pub use native::link_executable;

#[cfg(not(unix))]
mod unsupported {
//! Fail closed rather than claiming native link parity on untested targets.
use std::path::Path;
pub fn link_executable(_object: &[u8], _arity: usize, _output: &Path) -> Result<(), String> {
    Err("standalone native AOT build currently requires a Unix host".into())
}

}
#[cfg(not(unix))]
pub use unsupported::link_executable;
