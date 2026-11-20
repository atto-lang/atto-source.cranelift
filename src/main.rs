// Host-only STOP ledger; no guest supervisor lowering.
mod backend;
pub mod critical_gate;
mod frontend;
mod functional_pipelines;
mod isolate;
pub mod layout;
#[allow(dead_code)]
mod owned_region;
#[allow(dead_code)]
mod actor_isolation_access_v1;
mod state_region;
#[allow(dead_code)]
mod supervisor_stop;
pub mod uhl1;
use std::io::Read;

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
            Component::ParentDir | Component::Prefix(_) =>
                return Err("source parent traversal is unsupported".into()),
        }
    }
    if segments.is_empty() {
        return Err("source file path is empty".into());
    }

    for (index, segment) in segments.iter().enumerate() {
        let name = CString::new(segment.as_bytes())
            .map_err(|_| "source path contains NUL")?;
        let final_component = index + 1 == segments.len();
        let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW
            | if final_component { libc::O_NONBLOCK } else { libc::O_DIRECTORY };
        // SAFETY: directory is an owned live descriptor; name is a valid NUL-
        // terminated segment. File::from_raw_fd takes ownership of successful fd.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(format!("source path component denied: {}", std::io::Error::last_os_error()));
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
    if std::fs::symlink_metadata(path).map_err(|e| e.to_string())?
        .file_type().is_symlink() {
        return Err("source symlinks forbidden".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("source must be a regular file".into());
    }
    Ok(file)
}

fn run() -> Result<(), String> {
    let mut argv = std::env::args_os();
    let _ = argv.next();
    let mode = argv.next().ok_or(
        "usage: orescranelift run|ir|isolate|check <file.atto|file.ores> [i64 args...] or hotload-capabilities",
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
        return Err("full AOT object/executable emission is not implemented by the Cranelift scalar backend; refusing JIT fallback".into());
    }
    if mode != "run" && mode != "ir" && mode != "isolate" && mode != "check" {
        return Err("usage: orescranelift run|ir|isolate|check <file.atto|file.ores> [i64 args...] or hotload-capabilities".into());
    }
    let path = std::path::PathBuf::from(argv.next().ok_or("missing source file")?);
    // The stage-0 JIT ABI takes at most four i64 arguments. Reject surplus
    // argv BEFORE opening files, parsing source or allocating executable
    // memory. Reading at most five elements also avoids collecting an
    // unbounded attacker-supplied argument vector.
    let supplied: Vec<std::ffi::OsString> = argv.by_ref().take(5).collect();
    if (mode == "ir" || mode == "check") && !supplied.is_empty() {
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
    // Preserve the existing full stage-0 parser as authority. Only if it
    // rejects an explicitly pipeline-bearing source, attempt the separate
    // strict pure-int64 functional frontend before allocating JIT memory.
    // An invalid or effectful program cannot silently fall back to an
    // interpreted host evaluator.
    let parsed = match frontend::parse(source) {
        Ok(program) => program,
        Err(original) if source.contains("|>") || source.contains("<|")
            || source.contains(": int ->") =>
            functional_pipelines::parse(source)
                .map_err(|reason| format!("{reason} (stage-0: {original})"))?,
        Err(original) => return Err(original),
    };
    // Parser-only admission: never create a JIT module, write CLIF or execute.
    // A successful check is NOT a native compilation or guest-execution receipt.
    if mode == "check" {
        return Ok(());
    }
    if mode == "run" && values.len() != parsed.parameters.len() {
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
        let a = vm.create_isolate("tenant-a", 1, 1)?;
        let b = vm.create_isolate("tenant-b", 1, 1)?;
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
    } else {
        println!("{}", compiled.run(&values)?);
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("orescranelift: {e}");
        std::process::exit(1);
    }
}
