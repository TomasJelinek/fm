//! Opt-in bounded metadata trace in the broker's existing private profile.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
const MAX_EVENTS: usize = 2048;
static TRACE: OnceLock<Recorder<File>> = OnceLock::new();
#[derive(Clone, Copy)]
pub(crate) enum Event {
    InitializeStart,
    Initialized,
    ContextStart,
    ContextReady,
    Wheel,
    PaintBegin,
    PaintConverted,
    LayersCopied,
    FrameQueued,
    WireBegin,
    WireEnd,
    Closing,
}
impl Event {
    fn label(self) -> &'static str {
        match self {
            Self::InitializeStart => "initialize_start",
            Self::Initialized => "initialized",
            Self::ContextStart => "context_start",
            Self::ContextReady => "context_ready",
            Self::Wheel => "wheel",
            Self::PaintBegin => "paint_begin",
            Self::PaintConverted => "paint_converted",
            Self::LayersCopied => "layers_copied",
            Self::FrameQueued => "frame_queued",
            Self::WireBegin => "wire_begin",
            Self::WireEnd => "wire_end",
            Self::Closing => "closing",
        }
    }
}
struct Recorder<W> {
    writer: Mutex<W>,
    started: Instant,
    remaining: AtomicUsize,
}
impl<W: Write> Recorder<W> {
    fn record(&self, event: Event, first: u64, second: u64) {
        if self
            .remaining
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .is_err()
        {
            return;
        }
        let elapsed = self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writeln!(writer, "{elapsed},{},{first},{second}", event.label());
        }
    }
}
pub(crate) fn start(profile: &Path) {
    // Host supplies a fresh, private profile. Never replace an existing entry or
    // follow an existing diagnostic-file symlink. Failure only disables tracing.
    let Ok(mut file) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(profile.join("viewer-timing.csv"))
    else {
        return;
    };
    let utc = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    let _ = writeln!(file, "utc_start_us={utc}\nelapsed_us,event,first,second");
    let _ = TRACE.set(Recorder {
        writer: Mutex::new(file),
        started: Instant::now(),
        remaining: AtomicUsize::new(MAX_EVENTS),
    });
    record(Event::InitializeStart, 0, 0);
}
pub(crate) fn record(event: Event, first: u64, second: u64) {
    if let Some(trace) = TRACE.get() {
        trace.record(event, first, second);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_trace_is_strictly_bounded_under_concurrent_writers() {
        let trace = Recorder {
            writer: Mutex::new(Vec::new()),
            started: Instant::now(),
            remaining: AtomicUsize::new(MAX_EVENTS),
        };
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let trace = &trace;
                scope.spawn(move || {
                    for _ in 0..MAX_EVENTS {
                        trace.record(Event::PaintBegin, u64::MAX, u64::MAX);
                    }
                });
            }
        });
        let bytes = trace.writer.into_inner().unwrap();
        assert_eq!(
            bytes.iter().filter(|byte| **byte == b'\n').count(),
            MAX_EVENTS
        );
        assert!(bytes.len() < 256 * 1024);
    }
}
