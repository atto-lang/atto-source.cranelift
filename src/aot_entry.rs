//! Fixed, compiler-owned scalar entry shim. This is NOT a guest interpreter.
//! The program-specific AttoLang function is externally linked native code.
use std::process;

unsafe extern "C" {
    fn oreslang_scalar_entry(a: i64, b: i64, c: i64, d: i64, result: *mut i64) -> i32;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Atto's entry arity is embedded at compiler invocation time.
    if args.len().to_string() != env!("ATTO_AOT_ARITY") {
        eprintln!("atto-aot: runtime argument count mismatch");
        process::exit(64);
    }
    let mut padded = [0_i64; 4];
    for (index, input) in args.iter().enumerate() {
        let value = input.parse::<i64>().unwrap_or_else(|_| {
            eprintln!("atto-aot: invalid i64 argument");
            process::exit(65);
        });
        padded[index] = value;
    }
    let mut result = 0_i64;
    // SAFETY: exact ABI is generated in the same compilation, four i64 values
    // and a valid writable pointer; native object remains linked for process life.
    let status = unsafe {
        oreslang_scalar_entry(padded[0], padded[1], padded[2], padded[3], &mut result)
    };
    match status {
        0 => println!("{result}"),
        1 => {
            eprintln!("atto-aot: checked i64 arithmetic overflow");
            process::exit(1);
        }
        _ => {
            eprintln!("atto-aot: invalid native guest status");
            process::exit(70);
        }
    }
}
