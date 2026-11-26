// Host-only STOP ledger; no guest supervisor lowering.
#[allow(dead_code)]
mod actor_isolation_access_v1;
mod backend;
pub mod critical_gate;
mod frontend;
mod functional_pipelines;
mod isolate;
pub mod layout;
mod linked_native;
mod module_link;
#[allow(dead_code)]
mod owned_region;
#[allow(dead_code)]
mod result_int_parse_v1;
// Backend-local trusted host primitive; guest lowering pending.
mod owned_bytes_v1;
mod state_region;
#[allow(dead_code)]
mod supervisor_stop;
pub mod uhl1;
use std::io::{Read, Write};

// Resolve source paths through openat directory descriptors with O_NOFOLLOW at
// EVERY component, not just the leaf. A symlink swap between path validation
// and open must not redirect compiler input outside the expected tree.
#[cfg(unix)]
fn open_source_file(path: &std::path::Path) -> Result<std::fs::File, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    let mut directory = std::fs::File::open(if path.is_absolute() { "/" } else { "." })
        .map_err(|e| format!("source root: {e}"))?;
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => segments.push(name),
            // No ambiguous escape or normalization through ".." is accepted.
            Component::ParentDir | Component::Prefix(_) => {
                return Err("source parent traversal is unsupported".into());
            }
        }
    }
    if segments.is_empty() {
        return Err("source file path is empty".into());
    }

    for (index, segment) in segments.iter().enumerate() {
        let name = CString::new(segment.as_bytes()).map_err(|_| "source path contains NUL")?;
        let final_component = index + 1 == segments.len();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if final_component {
                libc::O_NONBLOCK
            } else {
                libc::O_DIRECTORY
            };
        // SAFETY: directory is an owned live descriptor; name is a valid NUL-
        // terminated segment. File::from_raw_fd takes ownership of successful fd.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if final_component && error.raw_os_error() == Some(libc::ELOOP) {
                return Err("source must be a regular file (symlinks forbidden)".into());
            }
            return Err(format!("source path component denied: {error}"));
        }
        directory = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    if !directory.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("source must be a regular file".into());
    }
    Ok(directory)
}

#[cfg(not(unix))]
fn open_source_file(path: &std::path::Path) -> Result<std::fs::File, String> {
    if std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("source symlinks forbidden".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("source must be a regular file".into());
    }
    Ok(file)
}

// Dispatch to the package-authorized linker only for a real import header.
// A line inside a multiline comment is not an import, and its text must not
// cause the otherwise valid single-unit scalar compiler to require a package
// manifest. This is a routing check, NOT an alternate import parser: the
// restricted module linker still validates every admitted header and forbids
// unsupported block-comment syntax within imported source graphs.
fn has_real_import_header(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut pos = 0;
    let mut block_depth = 0usize;
    let mut line_comment = false;
    let mut quote = false;
    let mut escaped = false;
    let mut at_line_start = true;
    while pos < bytes.len() {
        let current = bytes[pos];
        if current == b'\n' {
            line_comment = false;
            at_line_start = true;
            pos += 1;
            continue;
        }
        if line_comment {
            pos += 1;
            continue;
        }
        if block_depth > 0 {
            if bytes[pos..].starts_with(b"/*") {
                block_depth += 1;
                pos += 2;
            } else if bytes[pos..].starts_with(b"*/") {
                block_depth -= 1;
                pos += 2;
            } else {
                pos += 1;
            }
            continue;
        }
        if quote {
            if escaped {
                escaped = false;
            } else if current == b'\\' {
                escaped = true;
            } else if current == b'"' {
                quote = false;
            }
            pos += 1;
            continue;
        }
        if bytes[pos..].starts_with(b"//") {
            line_comment = true;
            pos += 2;
            continue;
        }
        if bytes[pos..].starts_with(b"/*") {
            block_depth = 1;
            pos += 2;
            continue;
        }
        if at_line_start && matches!(current, b' ' | b'\t' | b'\r') {
            pos += 1;
            continue;
        }
        if at_line_start && bytes[pos..].starts_with(b"import ") {
            return true;
        }
        at_line_start = false;
        if current == b'"' {
            quote = true;
        }
        pos += 1;
    }
    false
}

// Only trusted native scalar programs enter this ABI. Imported modules and
// standalone multi-function sources share exactly the same IR validator,
// checked call-status propagation and non-executing IR inspection path.
fn run_linked_native(
    mode: &str,
    values: &[i64],
    functions: &[functional_pipelines::Fn],
) -> Result<(), String> {
    if mode == "isolate" {
        return Err("E_LINK_ISOLATE_UNSUPPORTED".into());
    }
    if !matches!(mode, "check" | "ir" | "run") {
        return Err(
            "E_LINK_MODE_UNSUPPORTED: linked scalar graphs support check, ir, and run only".into(),
        );
    }
    linked_native::validate(functions)?;
    if mode == "check" {
        return Ok(());
    }
    if mode == "run" && !values.is_empty() {
        return Err("runtime argument count mismatch".into());
    }
    if mode == "ir" {
        print!("{}", linked_native::render_ir(functions)?);
        return Ok(());
    }
    let compiled = linked_native::compile(functions)?;
    println!("{}", compiled.run(&[])?);
    Ok(())
}

fn run() -> Result<(), String> {
    let mut argv = std::env::args_os();
    let _ = argv.next();
    let mode = argv.next().ok_or(
        "usage: orescranelift run|run-traced|ir|isolate|check|emit-object <file.atto|file.ores> [i64 args...] or hotload-capabilities",
    )?.into_string().map_err(|_| "invalid command mode encoding")?;
    if mode == "hotload-capabilities" {
        if argv.next().is_some() {
            return Err("hotload-capabilities accepts no source or guest arguments".into());
        }
        println!(
            "{}",
            uhl1::query(uhl1::ABI_VERSION).map_err(|_| "unsupported UHL-1 interface version")?
        );
        return Ok(());
    }
    // AOT may never silently fall back to the in-process stage-0 JIT.
    if mode == "aot" || mode == "full-aot" {
        return Err("full AOT executable linking is not implemented by the Cranelift scalar backend; refusing JIT fallback".into());
    }
    if !matches!(
        mode.as_str(),
        "run" | "run-traced" | "ir" | "isolate" | "check" | "emit-object"
    ) {
        return Err("usage: orescranelift run|run-traced|ir|isolate|check|emit-object <file.atto|file.ores> [i64 args...] or hotload-capabilities".into());
    }
    let path = std::path::PathBuf::from(argv.next().ok_or("missing source file")?);
    // The stage-0 JIT ABI takes at most four i64 arguments. Reject surplus
    // argv BEFORE opening files, parsing source or allocating executable
    // memory. Reading at most five elements also avoids collecting an
    // unbounded attacker-supplied argument vector.
    let supplied: Vec<std::ffi::OsString> = argv.by_ref().take(5).collect();
    if matches!(mode.as_str(), "ir" | "check" | "emit-object") && !supplied.is_empty() {
        return Err("ir/check mode does not accept runtime arguments".into());
    }
    if supplied.len() > 4 {
        return Err("too many runtime arguments".into());
    }
    // Parse all guest numeric values before reading source or allocating JIT
    // memory; OsString rejects non-UTF-8 without panicking the CLI iterator.
    let values = supplied
        .into_iter()
        .map(|v| {
            v.to_str()
                .ok_or_else(|| "invalid i64 argument".to_owned())?
                .parse::<i64>()
                .map_err(|_| "invalid i64 argument".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let file = open_source_file(&path)?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("source exceeds 1 MiB limit".into());
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| "source is not valid UTF-8")?;
    // Imported and standalone multi-function scalar graphs use separate
    // checked native functions. Legacy single-main and strict pure-pipeline
    // paths continue to lower to the stage-0 scalar Program shape.
    frontend::validate_source_envelope(source)?;
    let linked_source = has_real_import_header(source);
    if linked_source {
        // Module graphs must not enter the experimental host-mediated
        // TenantVM isolate lane, even before reading imported source files.
        if mode == "isolate" {
            return Err("E_LINK_ISOLATE_UNSUPPORTED".into());
        }
        let functions = module_link::parse(&path, source)?;
        return run_linked_native(&mode, &values, &functions);
    }
    // A valid standalone multi-function .atto translation unit must reach the
    // same real native call ABI used by imported public helpers, even when its
    // expressions happen to be safe to inline. The legacy pure-pipeline
    // expander is retained for single-function compatibility only.
    //
    // Parse the *entire* restricted source before choosing this path: lexical
    // counts of "fnc" (including strings/comments) cannot grant JIT admission.
    // Any linked-native validation failure is returned by run_linked_native;
    // it may never fall back to an inliner or a different semantic profile.
    // Keep explicit pipe/inline-arrow syntax on the existing pure pipeline lane.
    if path.extension().and_then(|ext| ext.to_str()) == Some("atto")
        && !source.contains("|>")
        && !source.contains("<|")
        && !source.contains(": int ->")
        && let Ok(functions) = functional_pipelines::parse_linked_native(source, &[])
        && functions.len() > 1
    {
        return run_linked_native(&mode, &values, &functions);
    }
    let parsed = match frontend::parse(source) {
        Ok(program) => program,
        Err(original)
            if source.contains("|>") || source.contains("<|") || source.contains(": int ->") =>
        {
            // The independent pure parser must share the outer admission gates.
            frontend::validate_source_envelope(source)?;
            functional_pipelines::parse(source)
                .map_err(|reason| format!("{reason} (stage-0: {original})"))?
        }
        Err(original) => return Err(original),
    };
    // Parser-only admission: never create a JIT module, write CLIF or execute.
    // A successful check is NOT a native compilation or guest-execution receipt.
    if mode == "check" {
        return Ok(());
    }
    if mode == "emit-object" {
        let object = backend::emit_object(&parsed)?;
        return std::io::stdout()
            .lock()
            .write_all(&object)
            .map_err(|e| e.to_string());
    }
    if matches!(mode.as_str(), "run" | "run-traced") && values.len() != parsed.parameters.len() {
        return Err("runtime argument count mismatch".into());
    }
    if mode == "isolate" && (parsed.parameters.len() != 2 || values.len() != 2) {
        return Err(
            "isolate mode expects a two-argument (old, delta) entry and two tenant deltas".into(),
        );
    }
    // IR inspection must not allocate/finalize executable JIT memory.
    if mode == "ir" {
        print!("{}", backend::render_ir(&parsed)?);
        return Ok(());
    }
    let compiled = backend::compile(&parsed)?;
    if mode == "isolate" {
        // Experimental *trusted* native scalar VM entry. The source must
        // accept (old, delta) i64; this is NOT a sandbox or actor scheduler.
        let vm = isolate::OresVM::new(2)?;
        let a = vm.create_tenant_vm("tenant-a", 1, 1)?;
        let b = vm.create_tenant_vm("tenant-b", 1, 1)?;
        vm.start(&a)?;
        vm.start(&b)?;
        let actor_a = vm.register_actor(&a, 1)?;
        let actor_b = vm.register_actor(&b, 1)?;
        // Ledger-only operation leases; no physical I/O is registered here.
        let pin_a = vm.pin_operation(&a, &actor_a)?;
        let pin_b = vm.pin_operation(&b, &actor_b)?;
        let before_a = vm.snapshot(&a)?;
        let before_b = vm.snapshot(&b)?;
        if before_a.phase != isolate::Phase::Running
            || before_b.phase != isolate::Phase::Running
            || before_a.generation == before_b.generation
            || before_a.actors != 1
            || before_b.actors != 1
            || before_a.active_pins != 1
            || before_b.active_pins != 1
            || before_a.active_guest_calls != 0
            || before_b.active_guest_calls != 0
        {
            return Err("isolate tenant admission receipt mismatch".into());
        }
        let left = compiled.run_for_isolate(&vm, &a, values[0])?;
        let right = compiled.run_for_isolate(&vm, &b, values[1])?;
        if vm.snapshot(&a)?.scalar_state != left || vm.snapshot(&b)?.scalar_state != right {
            return Err("compiled scalar guest state leaked between tenants".into());
        }
        vm.retire_operation(&a, &actor_a, &pin_a)?;
        vm.retire_operation(&b, &actor_b, &pin_b)?;
        vm.retire_actor(&a, &actor_a)?;
        vm.retire_actor(&b, &actor_b)?;
        vm.begin_shutdown(&a)?;
        vm.complete_shutdown(&a)?;
        vm.destroy(&a)?;
        vm.begin_shutdown(&b)?;
        vm.complete_shutdown(&b)?;
        vm.destroy(&b)?;
        println!("{left} {right}");
    } else if mode == "run-traced" {
        let (result, spans) = compiled.run_traced(&values);
        for span in spans {
            eprintln!(
                "trace span {} parent={} operation={:?}",
                span.id, span.parent, span.operation
            );
        }
        println!("{}", result?);
    } else {
        println!("{}", compiled.run(&values)?);
    }
    Ok(())
}
// Explicit internal one-entry host-native object session. No persistent
// cache, external package linkage, or JIT fallback.
#[derive(Default)]
struct IncrementalObjectSession { prior:Option<(String,Vec<u8>)> }
impl IncrementalObjectSession {
    fn build(&mut self,source:&str,cold:bool)->Result<(Vec<u8>,bool),String>{
        if !cold {if let Some((old,bytes))=&self.prior {if old==source{return Ok((bytes.clone(),true));}}}
        let parsed=frontend::parse(source)?;let bytes=backend::emit_object(&parsed)?;
        self.prior=Some((source.to_owned(),bytes.clone()));Ok((bytes,false))
    }
}
#[cfg(test)]
mod incremental_object_tests{
    use super::*;
    #[test]
    fn exact_native_object_reuse(){
        let a="pub fnc main(): int { return 41+1; }";let b="pub fnc main(): int { return 41+2; }";
        let mut s=IncrementalObjectSession::default();let (cold,hit)=s.build(a,true).unwrap();assert!(!hit);
        assert_eq!(cold,backend::emit_object(&frontend::parse(a).unwrap()).unwrap());
        let (warm,hit)=s.build(a,false).unwrap();assert!(hit);assert_eq!(warm,cold);
        let (edit,hit)=s.build(b,false).unwrap();assert!(!hit);assert_eq!(edit,backend::emit_object(&frontend::parse(b).unwrap()).unwrap());assert_ne!(cold,edit);
        let bad="pub fnc main(): int { return missing; }";assert_eq!(s.build(bad,false).unwrap_err(),frontend::parse(bad).unwrap_err());assert_eq!(s.build(b,false).unwrap(),(edit.clone(),true));
        let dir=std::env::temp_dir().join(format!("atto-clif-incremental-{}",std::process::id()));std::fs::create_dir(&dir).unwrap();
        let host=dir.join("host.c");std::fs::write(&host,"#include <stdint.h>\nextern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);\nint main(void){int64_t out=-1;int32_t rc=oreslang_scalar_entry(0,0,0,0,&out);return rc==0?(int)out:255;}\n").unwrap();
        for (bytes,expected) in [(cold,42),(warm,42),(edit,43)]{
            let obj=dir.join("guest.o");let exe=dir.join("guest");std::fs::write(&obj,bytes).unwrap();
            assert!(std::process::Command::new("cc").arg(&host).arg(&obj).arg("-o").arg(&exe).status().unwrap().success());
            assert_eq!(std::process::Command::new(&exe).status().unwrap().code(),Some(expected));
        }
        std::fs::remove_dir_all(dir).unwrap();println!("INCREMENTAL_CRANELIFT|actual-aot-guest=42/43|rollback");
    }
}
// Canonical compiler-cache host substrate, exact shared Rust source vectors.
mod sha256;
mod compiler_cache_key;
mod compiler_artifact_receipt;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod compiler_artifact_store;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod compiler_cache_key_policy;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod compiler_cache_ledger;
fn main() {
    if let Err(e) = run() {
        eprintln!("orescranelift: {e}");
        std::process::exit(1);
    }
}
