//! Opt-in debug timings for bounded text/document scrolling; disabled in release builds.
use std::time::Instant;

#[cfg(debug_assertions)]
fn clock() -> Option<Instant> {
    use std::sync::OnceLock;
    static START: OnceLock<Option<Instant>> = OnceLock::new();
    *START.get_or_init(|| std::env::var_os("MIDNIGHT_VIEWER_SCROLL_TRACE").map(|_| Instant::now()))
}

pub(super) fn begin(phase: &str, anchor: u64) -> Option<Instant> {
    #[cfg(debug_assertions)]
    if let Some(clock) = clock() {
        eprintln!(
            "scroll_phase phase={phase} at_ms={:.3} anchor={anchor}",
            clock.elapsed().as_secs_f64() * 1000.0
        );
        return Some(Instant::now());
    }
    let _ = (phase, anchor);
    None
}

pub(super) fn end(phase: &str, started: Option<Instant>) {
    if let Some(started) = started {
        eprintln!(
            "scroll_phase phase={phase} elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
