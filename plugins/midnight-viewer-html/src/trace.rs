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
    /// Decoded at stdin, before the bounded channel send; document and sequence.
    CommandReceived,
    /// Dequeued for dispatch, before lifecycle validation; document and sequence.
    CommandDispatch,
    /// One-command turn and CEF pump returned, before sleep; document and sequence.
    CommandTurnComplete,
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
            Self::CommandReceived => "command_received",
            Self::CommandDispatch => "command_dispatch",
            Self::CommandTurnComplete => "command_turn_complete",
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
    fn command_markers_preserve_generation_and_sequence_without_content() {
        let trace = Recorder {
            writer: Mutex::new(Vec::new()),
            started: Instant::now(),
            remaining: AtomicUsize::new(MAX_EVENTS),
        };
        for document in [7, u64::MAX] {
            trace.record(Event::CommandReceived, document, 11);
            trace.record(Event::CommandDispatch, document, 11);
            trace.record(Event::CommandTurnComplete, document, 11);
        }
        let bytes = trace.writer.into_inner().unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let fields: Vec<_> = text
            .lines()
            .map(|line| {
                let (elapsed, fields) = line.split_once(',').unwrap();
                assert!(elapsed.parse::<u64>().is_ok());
                fields
            })
            .collect();
        assert_eq!(
            fields,
            [
                "command_received,7,11",
                "command_dispatch,7,11",
                "command_turn_complete,7,11",
                "command_received,18446744073709551615,11",
                "command_dispatch,18446744073709551615,11",
                "command_turn_complete,18446744073709551615,11",
            ]
        );
    }

    #[test]
    fn exhausted_trace_does_not_fabricate_a_command_turn_completion() {
        let trace = Recorder {
            writer: Mutex::new(Vec::new()),
            started: Instant::now(),
            remaining: AtomicUsize::new(2),
        };
        trace.record(Event::CommandReceived, 7, 11);
        trace.record(Event::CommandDispatch, 7, 11);
        let admitted_bytes = trace.writer.lock().unwrap().len();
        trace.record(Event::CommandTurnComplete, 7, 11);
        trace.record(Event::CommandReceived, 7, 12);
        assert_eq!(trace.remaining.load(Ordering::Relaxed), 0);
        let bytes = trace.writer.into_inner().unwrap();
        assert_eq!(bytes.len(), admitted_bytes);
        let text = String::from_utf8(bytes).unwrap();
        let fields: Vec<_> = text
            .lines()
            .map(|line| line.split_once(',').unwrap().1)
            .collect();
        assert_eq!(fields, ["command_received,7,11", "command_dispatch,7,11"]);
    }

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
