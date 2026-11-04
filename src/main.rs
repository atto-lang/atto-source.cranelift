mod backend;
mod frontend;
mod state_region;
use std::io::Read;

fn run() -> Result<(), String> {
    let mut argv = std::env::args();
    let _ = argv.next();
    let mode = argv
        .next()
        .ok_or("usage: orescranelift run|ir <file.atto|file.ores> [i64 args...]")?;
    if mode != "run" && mode != "ir" {
        return Err("usage: orescranelift run|ir <file.atto|file.ores> [i64 args...]".into());
    }
    let path = argv.next().ok_or("missing source file")?;
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
