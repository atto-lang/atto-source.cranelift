mod aot_link;
mod backend;
mod frontend;
use std::io::{Read, Write};

fn run() -> Result<(), String> {
    let mut argv = std::env::args();
    let _ = argv.next();
    let mode = argv
        .next()
        .ok_or("usage: orescranelift run|ir|object|build <file.ores> [args|output]")?;
    if mode != "run" && mode != "ir" && mode != "object" && mode != "build" {
        return Err("usage: orescranelift run|ir|object|build <file.ores> [args|output]".into());
    }
    let path = argv.next().ok_or("missing source file")?;
    // Nonblocking admission is essential before descriptor-level validation:
    // File::open on a FIFO (including a symlink to a FIFO) otherwise parks.
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
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
    // AOT object emission must not instantiate a JITModule. Compile source
    // completely before creating the output so parse/IR rejection leaves none.
    if mode == "object" || mode == "build" {
        let output = argv.next().ok_or("object mode needs an output path")?;
        if argv.next().is_some() {
            return Err("object mode takes exactly one output path".into());
        }
        let bytes = backend::compile_object(&parsed)?;
        if mode == "build" {
            // The link shim is part of the compiler distribution; it only
            // parses i64 argv and calls already-compiled native guest code.
            // No JIT or guest interpreter is embedded into the executable.
            return aot_link::link_executable(
                &bytes, parsed.parameters.len(), std::path::Path::new(&output));
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|e| format!("cannot exclusively create AOT object: {e}"))?;
        // On failed writes, remove only the output we just created.
        if let Err(err) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&output);
            return Err(format!("AOT object write failed: {err}"));
        }
        return Ok(());
    }
    let values = argv
        .map(|v| {
            v.parse::<i64>()
                .map_err(|_| "invalid i64 argument".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if mode == "ir" && !values.is_empty() {
        return Err("ir mode does not accept arguments".into());
    }
    if mode == "run" && values.len() != parsed.parameters.len() {
        return Err("runtime argument count mismatch".into());
    }
    let compiled = backend::compile(&parsed)?;
    if mode == "ir" {
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
