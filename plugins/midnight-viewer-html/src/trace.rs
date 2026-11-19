//! Opt-in bounded metadata trace in the broker's existing private profile.
use std::{
    cell::RefCell,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
#[path = "trace/pump_window.rs"]
mod pump_window;

const MAX_EVENTS: usize = 2048;
static TRACE: OnceLock<Recorder<File>> = OnceLock::new();
#[derive(Clone, Copy)]
pub(crate) enum Event {
    InitializeStart,
    Initialized,
    /// Compiled experiment feature0/1 and observed global CEF switch0/1, unknown2.
    CompositingConfigured,
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
            Self::CompositingConfigured => "compositing_configured",
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
    /// Reserve the entire group, then write it under one writer lock. A missing
    /// group remains uncovered even when fewer than MAX_EVENTS rows were emitted.
    fn record_window(&self, summary: &pump_window::Summary) -> bool {
        const ROWS: usize = 9;
        if self
            .remaining
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(ROWS)
            })
            .is_err()
        {
            return false;
        }
        let sequence = summary.id.sequence;
        let fields = [
            ("wheel_paint_window", summary.id.document, sequence),
            ("wheel_paint_elapsed_us", sequence, summary.elapsed_us),
            ("wheel_paint_replaced", sequence, summary.replaced),
            ("wheel_pump_count", sequence, summary.pump_count),
            ("wheel_pump_gap_us", sequence, summary.pump_gap_us),
            (
                "wheel_pump_completed_us",
                sequence,
                summary.pump_completed_us,
            ),
            ("wheel_nav_completed_us", sequence, summary.nav_completed_us),
            (
                "wheel_pump_active",
                sequence,
                u64::from(summary.pump_active),
            ),
            ("wheel_pump_active_us", sequence, summary.pump_active_us),
        ];
        let mut group = std::io::Cursor::new([0u8; 1024]);
        for (event, first, second) in fields {
            if writeln!(group, "{},{event},{first},{second}", summary.at_us).is_err() {
                return false;
            }
        }
        let length = group.position() as usize;
        self.writer
            .lock()
            .is_ok_and(|mut writer| writer.write_all(&group.get_ref()[..length]).is_ok())
    }

    fn elapsed_us(&self) -> u64 {
        self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
    }

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
        let elapsed = self.elapsed_us();
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
// All callsites are on the broker main/CEF UI thread. Never hold a RefCell
// borrow across a CEF call: OnPaint can run inside do_message_loop_work.
thread_local! {
    static PUMP_WINDOW: RefCell<pump_window::Tracker> = RefCell::new(pump_window::Tracker::default());
}

fn update_window(update: impl FnOnce(&mut pump_window::Tracker, u64)) {
    if let Some(trace) = TRACE.get() {
        let at_us = trace.elapsed_us();
        PUMP_WINDOW.with(|tracker| update(&mut tracker.borrow_mut(), at_us));
    }
}

pub(crate) fn wheel(document: u64, sequence: u64) {
    update_window(|tracker, at_us| {
        tracker.wheel(pump_window::WheelId { document, sequence }, at_us);
    });
    record(Event::Wheel, sequence, 0);
}

pub(crate) fn reset_window() {
    update_window(|tracker, _| tracker.clear());
}

pub(crate) fn navigation_start() {
    update_window(pump_window::Tracker::navigation_start);
}

pub(crate) fn navigation_end() {
    update_window(pump_window::Tracker::navigation_end);
}

pub(crate) fn pump_start() {
    update_window(pump_window::Tracker::pump_start);
}

pub(crate) fn pump_end() {
    update_window(pump_window::Tracker::pump_end);
}

/// Capture before the Surface lock; invalid paints leave the pending window intact.
pub(crate) fn view_paint_entry() -> Option<u64> {
    TRACE.get().map(Recorder::elapsed_us)
}

pub(crate) fn valid_view_paint(
    document: u64,
    entry_us: Option<u64>,
) -> Option<pump_window::Summary> {
    let entry_us = entry_us?;
    PUMP_WINDOW.with(|tracker| tracker.borrow_mut().paint(document, entry_us, true))
}

/// Called after existing paint/queue markers. Every group timestamp is the saved
/// callback entry, so file order can differ from timestamp order. This is not ACK.
pub(crate) fn emit_paint_window(summary: Option<pump_window::Summary>) {
    if let Some(summary) = summary
        && let Some(trace) = TRACE.get()
    {
        let _ = trace.record_window(&summary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn summary() -> pump_window::Summary {
        pump_window::Summary {
            id: pump_window::WheelId {
                document: 7,
                sequence: 11,
            },
            at_us: 900,
            elapsed_us: 800,
            replaced: 2,
            pump_count: 3,
            pump_gap_us: 700,
            pump_completed_us: 50,
            nav_completed_us: 4,
            pump_active: true,
            pump_active_us: 600,
        }
    }

    #[test]
    fn summary_group_preserves_saved_entry_timestamp_and_exact_numeric_schema() {
        let trace = Recorder {
            writer: Mutex::new(Vec::new()),
            started: Instant::now(),
            remaining: AtomicUsize::new(MAX_EVENTS),
        };
        assert!(trace.record_window(&summary()));
        let text = String::from_utf8(trace.writer.into_inner().unwrap()).unwrap();
        assert_eq!(
            text,
            concat!(
                "900,wheel_paint_window,7,11\n",
                "900,wheel_paint_elapsed_us,11,800\n",
                "900,wheel_paint_replaced,11,2\n",
                "900,wheel_pump_count,11,3\n",
                "900,wheel_pump_gap_us,11,700\n",
                "900,wheel_pump_completed_us,11,50\n",
                "900,wheel_nav_completed_us,11,4\n",
                "900,wheel_pump_active,11,1\n",
                "900,wheel_pump_active_us,11,600\n",
            )
        );
    }

    #[test]
    fn summary_group_reserves_all_nine_rows_or_writes_nothing() {
        for budget in [0, 8, 9] {
            let trace = Recorder {
                writer: Mutex::new(Vec::new()),
                started: Instant::now(),
                remaining: AtomicUsize::new(budget),
            };
            assert_eq!(trace.record_window(&summary()), budget == 9);
            assert_eq!(
                trace.remaining.load(Ordering::Relaxed),
                if budget == 9 { 0 } else { budget }
            );
            let bytes = trace.writer.into_inner().unwrap();
            assert_eq!(
                bytes.iter().filter(|byte| **byte == b'\n').count(),
                if budget == 9 { 9 } else { 0 }
            );
        }
    }

    #[test]
    fn summary_group_uses_one_writer_call() {
        let trace = Recorder {
            writer: Mutex::new(Counting {
                writes: 0,
                bytes: Vec::new(),
            }),
            started: Instant::now(),
            remaining: AtomicUsize::new(9),
        };
        assert!(trace.record_window(&summary()));
        let writer = trace.writer.into_inner().unwrap();
        assert_eq!(writer.writes, 1);
        assert_eq!(
            writer.bytes.iter().filter(|byte| **byte == b'\n').count(),
            9
        );
    }

    #[test]
    fn maximum_numeric_summary_fits_one_stack_buffer() {
        let trace = Recorder {
            writer: Mutex::new(Vec::new()),
            started: Instant::now(),
            remaining: AtomicUsize::new(9),
        };
        let mut value = summary();
        value.at_us = u64::MAX;
        value.id = pump_window::WheelId {
            document: u64::MAX,
            sequence: u64::MAX,
        };
        value.elapsed_us = u64::MAX;
        value.replaced = u64::MAX;
        value.pump_count = u64::MAX;
        value.pump_gap_us = u64::MAX;
        value.pump_completed_us = u64::MAX;
        value.nav_completed_us = u64::MAX;
        value.pump_active_us = u64::MAX;
        assert!(trace.record_window(&value));
        let bytes = trace.writer.into_inner().unwrap();
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 9);
        assert!(bytes.len() < 1024);
    }

    #[test]
    fn concurrent_records_never_interleave_an_atomic_summary_group() {
        let trace = Recorder {
            writer: Mutex::new(Vec::new()),
            started: Instant::now(),
            remaining: AtomicUsize::new(MAX_EVENTS),
        };
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..MAX_EVENTS / 4 {
                    trace.record(Event::PaintBegin, 1, 1);
                }
            });
            for _ in 0..2 {
                let trace = &trace;
                scope.spawn(move || {
                    for _ in 0..MAX_EVENTS {
                        let _ = trace.record_window(&summary());
                    }
                });
            }
        });
        let bytes = trace.writer.into_inner().unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert!(lines.len() <= MAX_EVENTS);
        let expected = [
            "wheel_paint_window",
            "wheel_paint_elapsed_us",
            "wheel_paint_replaced",
            "wheel_pump_count",
            "wheel_pump_gap_us",
            "wheel_pump_completed_us",
            "wheel_nav_completed_us",
            "wheel_pump_active",
            "wheel_pump_active_us",
        ];
        let mut index = 0;
        let mut groups = 0;
        while index < lines.len() {
            if lines[index].split(',').nth(1) == Some("wheel_paint_window") {
                let group = &lines[index..index + 9];
                for (line, name) in group.iter().zip(expected) {
                    assert_eq!(line.split(',').nth(1), Some(name));
                    assert!(line.starts_with("900,"));
                }
                groups += 1;
                index += 9;
            } else {
                assert_eq!(lines[index].split(',').nth(1), Some("paint_begin"));
                index += 1;
            }
        }
        assert!(groups > 0);
    }

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
