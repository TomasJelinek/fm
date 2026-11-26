//! Opt-in viewer opening phase timestamps for startup measurements.
//!
//! Compiled only into debug builds and builds with the `open-trace` feature (the
//! `open_latency` probe); the application's release build contains no tracing. Set
//! `MIDNIGHT_VIEWER_OPEN_TRACE=1` to print one `viewer_phase` line per mark to standard
//! error. Times count from the first opening traced in the process, the same origin as
//! the viewer window's own opening trace. Results and method:
//! docs/verification/2026-11-26-viewer-startup.md.
#[cfg(any(debug_assertions, feature = "open-trace"))]
use std::sync::OnceLock;
use std::time::Instant;

#[cfg(any(debug_assertions, feature = "open-trace"))]
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("MIDNIGHT_VIEWER_OPEN_TRACE").is_some_and(|value| value == "1")
    })
}

/// The common origin of every opening timestamp, when tracing is enabled.
pub fn origin() -> Option<Instant> {
    #[cfg(any(debug_assertions, feature = "open-trace"))]
    if enabled() {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        return Some(*ORIGIN.get_or_init(Instant::now));
    }
    None
}

/// Record that `phase` was reached on the calling thread.
#[inline]
pub fn mark(phase: &'static str) {
    #[cfg(any(debug_assertions, feature = "open-trace"))]
    if let Some(origin) = origin() {
        use std::io::Write as _;
        let at = origin.elapsed().as_micros();
        let stderr = std::io::stderr();
        let _ = writeln!(stderr.lock(), "viewer_phase at_us={at} phase={phase}");
    }
    #[cfg(not(any(debug_assertions, feature = "open-trace")))]
    let _ = phase;
}

#[cfg(test)]
mod tests {
    #[test]
    fn tracing_stays_off_unless_requested() {
        if std::env::var_os("MIDNIGHT_VIEWER_OPEN_TRACE").is_some_and(|value| value == "1") {
            assert!(super::origin().is_some());
        } else {
            assert!(super::origin().is_none());
            // A mark without tracing writes nothing and has no origin to start.
            super::mark("test_phase");
            assert!(super::origin().is_none());
        }
    }
}
