use std::cell::RefCell;

pub(super) const BEGIN: &str = "ores_native_telemetry_span_begin";
pub(super) const END: &str = "ores_native_telemetry_span_end";

pub(crate) struct Span {
    pub id: u64,
    pub parent: u64,
    pub operation: String,
}

struct Recorder {
    enabled: bool,
    last_id: u64,
    active: Vec<Span>,
    completed: Vec<Span>,
}

thread_local! {
    static RECORDER: RefCell<Recorder> = const { RefCell::new(Recorder {
        enabled: false, last_id: 0, active: Vec::new(), completed: Vec::new(),
    }) };
}

pub(super) fn record<T>(run: impl FnOnce() -> T) -> (T, Vec<Span>) {
    RECORDER.with_borrow_mut(|recorder| recorder.enabled = true);
    let outcome = run();
    let completed = RECORDER.with_borrow_mut(|recorder| {
        recorder.enabled = false;
        std::mem::take(&mut recorder.completed)
    });
    (outcome, completed)
}

/// The trusted JIT compiler supplies a readable immutable literal range.
pub(super) unsafe extern "C" fn begin(operation: *const u8, length: u64) -> u64 {
    RECORDER.with_borrow_mut(|recorder| {
        if !recorder.enabled || recorder.active.len() == 64 || operation.is_null()
            || !(1..=256).contains(&length)
        {
            return 0;
        }
        let Some(id) = recorder.last_id.checked_add(1) else { return 0 };
        // SAFETY: the only bound callers are compiler-generated native
        // functions with a statically defined literal and exact byte length.
        let bytes = unsafe { std::slice::from_raw_parts(operation, length as usize) };
        let Ok(operation) = std::str::from_utf8(bytes) else { return 0 };
        let parent = recorder.active.last().map_or(0, |span| span.id);
        recorder.last_id = id;
        recorder.active.push(Span { id, parent, operation: operation.to_owned() });
        id
    })
}

pub(super) extern "C" fn end(token: u64) {
    if token == 0 {
        return;
    }
    RECORDER.with_borrow_mut(|recorder| {
        if recorder.active.last().is_some_and(|span| span.id == token) {
            let span = recorder.active.pop().unwrap();
            if recorder.completed.len() < 1024 {
                recorder.completed.push(span);
            }
        }
    });
}
