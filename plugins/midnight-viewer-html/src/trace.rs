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
    /// Command batch and CEF pump returned; last command's document and sequence.
    CommandTurnComplete,
    Wheel,
    PaintBegin,
    PaintConverted,
    /// Frame pixels ready to queue: the moved paint, or a popup composition copy.
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
        // Format outside the lock and issue a single write: an unbuffered File would
        // otherwise make one syscall per formatted piece inside the measured interval.
        let mut line = std::io::Cursor::new([0u8; 128]);
        if writeln!(line, "{elapsed},{},{first},{second}", event.label()).is_err() {
            return;
        }
        let length = line.position() as usize;
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(&line.get_ref()[..length]);
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
    fn each_record_is_a_single_write_outside_the_measured_paint_path() {
        struct Counting {
            writes: usize,
            bytes: Vec<u8>,
        }
        impl Write for Counting {
            fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(buffer);
                Ok(buffer.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let trace = Recorder {
            writer: Mutex::new(Counting {
                writes: 0,
                bytes: Vec::new(),
            }),
            started: Instant::now(),
            remaining: AtomicUsize::new(MAX_EVENTS),
        };
        trace.record(Event::CommandTurnComplete, u64::MAX, u64::MAX);
        trace.record(Event::WireEnd, 7, 33_177_600);
        let counting = trace.writer.into_inner().unwrap();
        assert_eq!(counting.writes, 2);
        let text = String::from_utf8(counting.bytes).unwrap();
        let fields: Vec<_> = text
            .lines()
            .map(|line| line.split_once(',').unwrap().1)
            .collect();
        assert_eq!(
            fields,
            [
                "command_turn_complete,18446744073709551615,18446744073709551615",
                "wire_end,7,33177600"
            ]
        );
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
