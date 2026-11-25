use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const BIDI: [char; 12] = [
    '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
    '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
];

fn invoke(source: &str) -> std::process::Output {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("ores-bidi-{}-{id}.atto", std::process::id()));
    std::fs::write(&path, source).expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_orescranelift"))
        .arg("ir")
        .arg(&path)
        .output()
        .expect("run real Cranelift frontend");
    std::fs::remove_file(path).expect("remove fixture");
    output
}

#[test]
fn all_bidi_controls_fail_closed_before_ir_output() {
    let safe = "fnc main():int{return 42;}";
    for control in BIDI {
        for source in [
            format!("{safe} // {control}"),
            format!("/* {control} */ {safe}"),
        ] {
            let result = invoke(&source);
            assert!(
                !result.status.success(),
                "admitted U+{:04X}",
                control as u32
            );
            assert!(result.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&result.stderr)
                    .contains("source contains bidirectional Unicode control")
            );
        }
    }
}
