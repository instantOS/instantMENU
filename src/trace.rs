//! Opt-in startup tracing (`INSTANTMENU_TRACE=1`). Prints monotonic
//! millisecond marks to stderr; compiled into normal builds but the enabled
//! flag is read once, so the hot path costs one atomic load after startup.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static ENABLED: OnceLock<bool> = OnceLock::new();

pub fn init() {
    let _ = START.set(Instant::now());
    let _ = ENABLED.set(std::env::var_os("INSTANTMENU_TRACE").is_some());
    mark("process start");
}

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("INSTANTMENU_TRACE").is_some())
}

/// Print a mark when tracing is enabled. The label is a plain &'static str so
/// calling this from the render path does not allocate.
pub fn mark(label: &str) {
    if !enabled() {
        return;
    }
    let elapsed = START.get().map(Instant::elapsed).unwrap_or_default();
    eprintln!("TRACE {:9.3}ms {label}", elapsed.as_secs_f64() * 1000.0);
}
