use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// A program whose streams are in memory and that exits once both are read, or when
/// ended (politely only when `polite`).
struct Fake {
    streams: Option<CapturedStreams>,
    exit: Option<ProgramExit>,
    drained: Arc<AtomicUsize>,
    exit_code: i64,
    ended: Arc<AtomicBool>,
    polite: bool,
    terminations: Arc<Mutex<Vec<bool>>>,
}

/// Bytes, then the end of the stream (counted in `drained`); or, when `endless`, no end
/// until `ended`.
struct Stream {
    chunks: std::collections::VecDeque<Vec<u8>>,
    drained: Arc<AtomicUsize>,
    endless: Option<Arc<AtomicBool>>,
}
impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(mut chunk) = self.chunks.pop_front() {
            let length = chunk.len().min(buffer.len());
            buffer[..length].copy_from_slice(&chunk[..length]);
            if length < chunk.len() {
                self.chunks.push_front(chunk.split_off(length));
            }
            return Ok(length);
        }
        if let Some(ended) = &self.endless {
            while !ended.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
        }
        self.drained.fetch_add(1, Ordering::AcqRel);
        self.endless = None;
        Ok(0)
    }
}

impl Fake {
    fn new(stdout: Vec<Vec<u8>>, stderr: Vec<Vec<u8>>, exit_code: i64, endless: bool) -> Self {
        let drained = Arc::new(AtomicUsize::new(0));
        let ended = Arc::new(AtomicBool::new(false));
        let stream = |chunks: Vec<Vec<u8>>| -> Box<dyn Read + Send> {
            Box::new(Stream {
                chunks: chunks.into(),
                drained: drained.clone(),
                endless: endless.then(|| ended.clone()),
            })
        };
        Self {
            streams: Some(CapturedStreams {
                stdout: stream(stdout),
                stderr: stream(stderr),
            }),
            exit: None,
            drained: drained.clone(),
            exit_code,
            ended,
            polite: true,
            terminations: Arc::default(),
        }
    }
}

impl CapturedUserProgram for Fake {
    fn id(&self) -> u32 {
        7
    }
    fn take_streams(&mut self) -> Option<CapturedStreams> {
        self.streams.take()
    }
    fn try_wait(&mut self) -> io::Result<Option<ProgramExit>> {
        if self.exit.is_none()
            && (self.ended.load(Ordering::Acquire) || self.drained.load(Ordering::Acquire) == 2)
        {
            self.exit = Some(ProgramExit::Code(self.exit_code));
        }
        Ok(self.exit)
    }
    fn terminate_tree(&mut self, force: bool) -> io::Result<()> {
        self.terminations.lock().unwrap().push(force);
        if force || self.polite {
            self.exit_code = 1;
            self.ended.store(true, Ordering::Release);
        }
        Ok(())
    }
}

fn new_session(planned: u64) -> Arc<UserCommandOutputSession> {
    UserCommandOutputSession::new(
        UserCommandRequestId(1),
        UserCommandId::new("tool").unwrap(),
        "Tool".into(),
        planned,
    )
}

fn publisher() -> (Publish, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    (
        Arc::new(move |_: &Arc<UserCommandOutputSession>| {
            counted.fetch_add(1, Ordering::AcqRel);
            true
        }),
        count,
    )
}

fn texts(view: &OutputView, stream: OutputStream) -> Vec<String> {
    view.lines
        .iter()
        .filter(|line| line.stream == stream)
        .map(|line| line.text.clone())
        .collect()
}

#[test]
fn streams_arrive_as_clean_lines_with_the_exit_code() {
    let program = Fake::new(
        vec![
            b"plain\n\x1b[1;31mred\x1b[0m and \x1b]0;title\x07text\n".to_vec(),
            b"progress  10%\rprogress  50%\r".to_vec(),
            b"progress 100%\r\nbad \xff\xfe bytes\n\tindented\nno newline".to_vec(),
        ],
        vec![b"warning: one\n".to_vec(), b"warning: two".to_vec()],
        3,
        false,
    );
    let session = new_session(1);
    let (publish, _) = publisher();
    let exit = supervise(
        Box::new(program),
        &session,
        0,
        &Cancellation::default(),
        &publish,
    );
    assert_eq!(exit, Some(ProgramExit::Code(3)));
    let view = session.read_since(0);
    assert_eq!(
        texts(&view, OutputStream::Stdout),
        [
            "plain",
            "red and text",
            "progress 100%",
            "bad \u{fffd}\u{fffd} bytes",
            "        indented",
            "no newline",
        ]
    );
    assert_eq!(
        texts(&view, OutputStream::Stderr),
        ["warning: one", "warning: two"]
    );
    let notices = texts(&view, OutputStream::Notice);
    assert_eq!(notices.len(), 1);
    assert!(notices[0].starts_with("Exit code 3 · "), "{notices:?}");
    assert!(view.partial.is_empty(), "every stream ended");
}

#[test]
fn a_flood_of_output_stays_within_the_bounds() {
    let line = format!("{}\n", "z".repeat(99)).into_bytes();
    // 64 MiB in 64 KiB chunks: far beyond what a window keeps.
    let chunk: Vec<u8> = line.iter().copied().cycle().take(64 * 1024).collect();
    let program = Fake::new(vec![chunk; 1024], Vec::new(), 0, false);
    let session = new_session(1);
    let (publish, published) = publisher();
    let exit = supervise(
        Box::new(program),
        &session,
        0,
        &Cancellation::default(),
        &publish,
    );
    assert_eq!(exit, Some(ProgramExit::Code(0)));
    let view = session.read_since(0);
    assert!(view.lines.len() <= MAX_OUTPUT_LINES);
    assert!(view.lines.iter().map(|line| line.text.len()).sum::<usize>() <= MAX_OUTPUT_BYTES);
    assert!(view.dropped > 600_000, "dropped {}", view.dropped);
    assert_eq!(view.first, view.dropped);
    // Never read by a UI: one notification, then none until it reads.
    assert_eq!(published.load(Ordering::Acquire), 1);
}

#[test]
fn stop_ends_the_tree_and_kills_it_when_asked_politely_in_vain() {
    let session = new_session(1);
    let (publish, _) = publisher();
    let program = Fake::new(vec![b"working\n".to_vec()], Vec::new(), 0, true);
    let terminations = program.terminations.clone();
    let stopper = session.clone();
    let stop = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        stopper.request_stop();
    });
    let exit = supervise(
        Box::new(program),
        &session,
        0,
        &Cancellation::default(),
        &publish,
    );
    stop.join().unwrap();
    assert_eq!(exit, Some(ProgramExit::Code(1)));
    assert_eq!(*terminations.lock().unwrap(), [false]);
    let view = session.read_since(0);
    assert_eq!(texts(&view, OutputStream::Stdout), ["working"]);
    assert!(texts(&view, OutputStream::Notice)[0].starts_with("Stopped · Exit code 1"));
    assert!(
        view.runs.is_empty(),
        "supervise records no run it did not begin"
    );

    // A program that ignores the polite request is killed after the grace period;
    // cancelling the request (closing the app) stops like Stop.
    let session = new_session(1);
    let mut program = Fake::new(Vec::new(), Vec::new(), 0, true);
    program.polite = false;
    let terminations = program.terminations.clone();
    let cancel = Cancellation::default();
    cancel.cancel();
    let started = Instant::now();
    let exit = supervise(Box::new(program), &session, 0, &cancel, &publish);
    assert_eq!(exit, Some(ProgramExit::Code(1)));
    assert_eq!(*terminations.lock().unwrap(), [false, true]);
    assert!(started.elapsed() >= STOP_GRACE);
}

#[test]
fn a_notification_that_could_not_be_queued_is_sent_again() {
    let session = new_session(1);
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let refusing: Publish = Arc::new(move |_: &Arc<UserCommandOutputSession>| {
        counted.fetch_add(1, Ordering::AcqRel) > 0
    });
    session.append(OutputStream::Stdout, vec!["one".into()], None);
    notify(&session, &refusing);
    assert_eq!(attempts.load(Ordering::Acquire), 1, "the queue was full");
    notify(&session, &refusing);
    assert_eq!(attempts.load(Ordering::Acquire), 2, "tried again");
    notify(&session, &refusing);
    assert_eq!(
        attempts.load(Ordering::Acquire),
        2,
        "sent; the UI has not read"
    );
}
