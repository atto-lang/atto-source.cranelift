// Host-only STOP ledger; no guest supervisor lowering.
mod backend;
pub mod critical_gate;
mod frontend;
mod isolate;
#[allow(dead_code)]
mod actor_isolation_access_v1;
mod state_region;
#[allow(dead_code)]
mod supervisor_stop;
pub mod uhl1;
use std::io::Read;

fn run() -> Result<(), String> {
    let mut argv = std::env::args();
    let _ = argv.next();
    let mode = argv.next().ok_or(
        "usage: orescranelift run|ir|isolate <file.atto|file.ores> [i64 args...] or hotload-capabilities",
    )?;
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
    if mode != "run" && mode != "ir" && mode != "isolate" {
        return Err("usage: orescranelift run|ir|isolate <file.atto|file.ores> [i64 args...] or hotload-capabilities".into());
    }
    let path = argv.next().ok_or("missing source file")?;
    // The stage-0 JIT ABI takes at most four i64 arguments. Reject surplus
    // argv BEFORE opening files, parsing source or allocating executable
    // memory. Reading at most five elements also avoids collecting an
    // unbounded attacker-supplied argument vector.
    let supplied: Vec<String> = argv.by_ref().take(5).collect();
    if mode == "ir" && !supplied.is_empty() {
        return Err("ir mode does not accept arguments".into());
    }
    if supplied.len() > 4 {
        return Err("too many runtime arguments".into());
    }
    // Nonblocking admission is essential before descriptor-level validation:
    // File::open on a FIFO (including a symlink to a FIFO) otherwise parks.
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Reject a symlink leaf to prevent a caller-controlled source path from
        // resolving to a different file (or a special device) at open time.
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    // Reject a final-component symlink before opening. On Unix O_NOFOLLOW
    // closes the check/open race, including a symlink swapped in afterward.
    // Intermediate path-component symlinks remain outside this boundary.
    if std::fs::symlink_metadata(&path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("source must be a regular file (symlinks forbidden)".into());
    }
    let file = options.open(&path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("source must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("source exceeds 1 MiB limit".into());
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| "source is not valid UTF-8")?;
    let parsed = frontend::parse(source)?;
    let values = supplied
        .into_iter()
        .map(|v| {
            v.parse::<i64>()
                .map_err(|_| "invalid i64 argument".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if mode == "run" && values.len() != parsed.parameters.len() {
        return Err("runtime argument count mismatch".into());
    }
    if mode == "isolate" && (parsed.parameters.len() != 2 || values.len() != 2) {
        return Err(
            "isolate mode expects a two-argument (old, delta) entry and two tenant deltas".into(),
        );
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
    } else if mode == "ir" {
        print!("{}", compiled.clif);
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
