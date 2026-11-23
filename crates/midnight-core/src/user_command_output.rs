//! Captured Run directly output: terminal-control cleanup, a bounded line log and the
//! session a backend worker writes and the UI reads. No process or file I/O here.
use crate::{UserCommandId, UserCommandRequestId};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fmt, io,
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

/// Retained lines per output window; older lines are dropped and counted.
pub const MAX_OUTPUT_LINES: usize = 50_000;
/// Retained text per output window, in bytes of decoded text.
pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// A longer line without a line break is wrapped into several lines.
pub const MAX_OUTPUT_LINE_BYTES: usize = 16 * 1024;
/// Output windows a session may have open at once.
pub const MAX_OUTPUT_SESSIONS: usize = 16;
/// How often a running session tells the UI that it has new output, at most.
pub const OUTPUT_PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

/// The stream a line came from; `Notice` lines are the window's own (a run starting,
/// its exit code).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputStream {
    Stdout,
    Stderr,
    Notice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: OutputStream,
    pub text: String,
}

/// Turns one complete line of a program's bytes into text. Platforms supply one that
/// knows their legacy console encoding; every decoder accepts arbitrary bytes.
pub type OutputDecoder = fn(&[u8]) -> String;

/// UTF-8, with invalid sequences replaced (U+FFFD).
pub fn decode_utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Tabs become spaces to the next multiple of eight columns, as a terminal shows them.
pub fn expand_tabs(text: String) -> String {
    if !text.contains('\t') {
        return text;
    }
    let mut expanded = String::with_capacity(text.len() + 8);
    let mut column = 0usize;
    for character in text.chars() {
        if character == '\t' {
            let spaces = 8 - column % 8;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(character);
            column += 1;
        }
    }
    expanded
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    /// After ESC: an intermediate byte, `[` (CSI), a string introducer or a final byte.
    Start,
    Csi,
    /// OSC, DCS, SOS, PM or APC text up to BEL or ST; `true` right after an ESC.
    Text(bool),
}

/// Terminal output as lines: ANSI/VT escape sequences (colours, cursor movement, window
/// titles) are removed, a carriage return not followed by a line feed starts the line
/// again (progress counters rewrite their line), backspace removes the previous
/// character, other control characters are dropped and a line longer than
/// [`MAX_OUTPUT_LINE_BYTES`] is wrapped. Bytes stay bytes: decoding happens per line.
#[derive(Debug, Default)]
pub struct TerminalLines {
    line: Vec<u8>,
    escape: Escape,
    escape_bytes: usize,
    /// A carriage return was seen: the next character replaces the line.
    carriage: bool,
}

impl TerminalLines {
    /// Feed bytes of the stream; each complete line is passed to `line` without its
    /// terminator.
    pub fn feed(&mut self, bytes: &[u8], mut line: impl FnMut(Vec<u8>)) {
        for &byte in bytes {
            if self.escape(byte) {
                continue;
            }
            match byte {
                0x1B => {
                    self.escape = Escape::Start;
                    self.escape_bytes = 0;
                }
                b'\n' => {
                    self.carriage = false;
                    line(std::mem::take(&mut self.line));
                }
                b'\r' => self.carriage = true,
                0x08 => {
                    // Remove one character: its UTF-8 continuation bytes too.
                    while let Some(last) = self.line.pop() {
                        if last & 0xC0 != 0x80 {
                            break;
                        }
                    }
                }
                b'\t' => self.push(byte, &mut line),
                0x00..=0x1F | 0x7F => {}
                _ => self.push(byte, &mut line),
            }
        }
    }

    fn push(&mut self, byte: u8, line: &mut impl FnMut(Vec<u8>)) {
        if self.carriage {
            self.carriage = false;
            self.line.clear();
        }
        self.line.push(byte);
        if self.line.len() >= MAX_OUTPUT_LINE_BYTES {
            // Wrap after a complete UTF-8 character: an incomplete one at the end moves
            // to the next line. Other encodings may split a character here.
            let length = self.line.len();
            let cut = (length.saturating_sub(3)..length)
                .rev()
                .find(|&at| self.line[at] & 0xC0 != 0x80)
                .filter(|&at| at > 0 && at + utf8_length(self.line[at]) > length)
                .unwrap_or(length);
            let rest = self.line.split_off(cut);
            line(std::mem::replace(&mut self.line, rest));
        }
    }

    /// Whether `byte` belongs to an escape sequence (and was consumed).
    fn escape(&mut self, byte: u8) -> bool {
        let state = self.escape;
        if state == Escape::None {
            return false;
        }
        self.escape_bytes += 1;
        self.escape = match state {
            Escape::Start => match byte {
                0x20..=0x2F if self.escape_bytes < 16 => Escape::Start,
                b'[' => Escape::Csi,
                b']' | b'P' | b'X' | b'^' | b'_' => Escape::Text(false),
                _ => Escape::None,
            },
            Escape::Csi if (0x40..=0x7E).contains(&byte) || self.escape_bytes > 256 => Escape::None,
            Escape::Csi if byte == 0x1B => {
                self.escape_bytes = 0;
                Escape::Start
            }
            Escape::Csi => Escape::Csi,
            Escape::Text(_) if byte == 0x07 || self.escape_bytes > 4096 => Escape::None,
            Escape::Text(true) if byte == b'\\' => Escape::None,
            Escape::Text(_) => Escape::Text(byte == 0x1B),
            Escape::None => Escape::None,
        };
        true
    }

    /// The unterminated line so far: what a terminal shows on its last row.
    pub fn partial(&self) -> &[u8] {
        &self.line
    }

    /// The last line at the end of the stream, if it has any text.
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        self.escape = Escape::None;
        self.carriage = false;
        (!self.line.is_empty()).then(|| std::mem::take(&mut self.line))
    }
}

/// The length of the UTF-8 sequence a byte starts (1 for anything else).
fn utf8_length(lead: u8) -> usize {
    match lead {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// A partial line's text: an incomplete UTF-8 character at its end (more bytes are
/// coming) is left out rather than shown as a replacement character.
pub fn decode_partial(bytes: &[u8], decoder: OutputDecoder) -> String {
    let complete = match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => bytes.len(),
    };
    expand_tabs(decoder(&bytes[..complete]))
}

/// Retained output lines, oldest first, bounded by [`MAX_OUTPUT_LINES`] and
/// [`MAX_OUTPUT_BYTES`]. Every line has a sequence number; readers ask for the lines
/// after the last one they have.
#[derive(Debug, Default)]
pub struct OutputLog {
    lines: VecDeque<OutputLine>,
    bytes: usize,
    /// Sequence number of `lines[0]`.
    first: u64,
    /// Lines dropped to stay within the bounds since the last Clear.
    dropped: u64,
    /// The unterminated last line of stdout and of stderr.
    partial: [Option<String>; 2],
}

impl OutputLog {
    pub fn push(&mut self, line: OutputLine) {
        self.bytes += line.text.len();
        self.lines.push_back(line);
        while self.lines.len() > MAX_OUTPUT_LINES
            || (self.bytes > MAX_OUTPUT_BYTES && self.lines.len() > 1)
        {
            if let Some(old) = self.lines.pop_front() {
                self.bytes -= old.text.len();
                self.first += 1;
                self.dropped += 1;
            }
        }
    }

    pub fn set_partial(&mut self, stream: OutputStream, text: Option<String>) {
        if let Some(slot) = partial_slot(stream) {
            self.partial[slot] = text.filter(|text| !text.is_empty());
        }
    }

    /// Sequence number of the first retained line.
    pub fn first(&self) -> u64 {
        self.first
    }

    /// Sequence number the next line gets.
    pub fn next(&self) -> u64 {
        self.first + self.lines.len() as u64
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Lines with a sequence number from `from` on (clamped to the retained ones).
    pub fn since(&self, from: u64) -> impl Iterator<Item = &OutputLine> {
        let skip = from.saturating_sub(self.first).min(self.lines.len() as u64) as usize;
        self.lines.iter().skip(skip)
    }

    /// The unterminated last lines, stdout first.
    pub fn partials(&self) -> impl Iterator<Item = OutputLine> + '_ {
        [OutputStream::Stdout, OutputStream::Stderr]
            .into_iter()
            .zip(&self.partial)
            .filter_map(|(stream, text)| {
                text.as_ref().map(|text| OutputLine {
                    stream,
                    text: text.clone(),
                })
            })
    }

    /// Forget every retained line; unterminated lines stay (they are still being written).
    pub fn clear(&mut self) {
        self.first = self.next();
        self.lines.clear();
        self.bytes = 0;
        self.dropped = 0;
    }

    /// Everything retained as plain text, one line per line, unterminated lines last.
    pub fn text(&self) -> String {
        let mut text = String::with_capacity(self.bytes + self.lines.len() + 64);
        for line in self.lines.iter().cloned().chain(self.partials()) {
            text.push_str(&line.text);
            text.push('\n');
        }
        text
    }
}

fn partial_slot(stream: OutputStream) -> Option<usize> {
    match stream {
        OutputStream::Stdout => Some(0),
        OutputStream::Stderr => Some(1),
        OutputStream::Notice => None,
    }
}

/// How a program ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProgramExit {
    /// The exit status (Windows: the 32-bit exit code, never negative).
    Code(i64),
    /// Ended by a POSIX signal.
    Signal(i32),
}

impl ProgramExit {
    pub fn success(self) -> bool {
        self == Self::Code(0)
    }

    /// "Exit code 1", "Exit code 3221225786 (0xC000013A)", "Ended by signal 15".
    pub fn describe(self) -> String {
        match self {
            Self::Code(code) if (256..=i64::from(u32::MAX)).contains(&code) => {
                format!("Exit code {code} (0x{code:08X})")
            }
            Self::Code(code) => format!("Exit code {code}"),
            Self::Signal(signal) => format!("Ended by signal {signal}"),
        }
    }
}

/// "1.2 s", "2 min 05 s", "1 h 02 min".
pub fn describe_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{}.{} s", seconds, duration.subsec_millis() / 100)
    } else if seconds < 3600 {
        format!("{} min {:02} s", seconds / 60, seconds % 60)
    } else {
        format!("{} h {:02} min", seconds / 3600, seconds / 60 % 60)
    }
}

/// The program and its arguments as one line, for display only: an argument that is
/// empty or contains a space, tab or quote is shown in double quotes, inner quotes and
/// backslashes before them escaped. Nothing parses this text; the program receives
/// its arguments separately.
pub fn display_command_line(program: &Path, arguments: &[OsString]) -> String {
    let mut line = String::new();
    for (index, value) in std::iter::once(program.as_os_str())
        .chain(arguments.iter().map(OsString::as_os_str))
        .enumerate()
    {
        if index > 0 {
            line.push(' ');
        }
        let value = value.to_string_lossy();
        if !value.is_empty() && !value.contains([' ', '\t', '"']) {
            line.push_str(&value);
            continue;
        }
        line.push('"');
        let mut backslashes = 0usize;
        for character in value.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    line.push('"');
                    backslashes = 0;
                    continue;
                }
                _ => {}
            }
            if character != '\\' {
                line.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                line.push(character);
            }
        }
        line.extend(std::iter::repeat_n('\\', backslashes * 2));
        line.push('"');
    }
    line
}

/// One started process of an output session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputRun {
    /// Zero-based position in the activation's plan.
    pub index: u64,
    pub command_line: String,
    pub directory: String,
    pub started: SystemTime,
    pub process_id: u32,
    pub exit: Option<ProgramExit>,
    pub duration: Option<Duration>,
    /// Stop ended this run.
    pub stopped: bool,
}

/// The end of an output session: every planned run has ended or will not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputFinish {
    pub cancelled: bool,
    /// Why the remaining runs did not start, already described for people.
    pub error: Option<String>,
}

/// What changed since a reader's last look ([`UserCommandOutputSession::read_since`]).
#[derive(Clone, Debug, Default)]
pub struct OutputView {
    /// Sequence number of the first retained line; rows before it are gone.
    pub first: u64,
    /// The new lines, from the reader's position (or `first`) on.
    pub lines: Vec<OutputLine>,
    /// Sequence number after the last retained line.
    pub next: u64,
    pub dropped: u64,
    pub partial: Vec<OutputLine>,
    pub runs: Vec<OutputRun>,
    pub planned: u64,
    pub finished: Option<OutputFinish>,
    pub stopping: bool,
}

#[derive(Debug, Default)]
struct SessionState {
    log: OutputLog,
    runs: Vec<OutputRun>,
    planned: u64,
    finished: Option<OutputFinish>,
}

/// One Custom Command activation shown in an output window. The backend appends and
/// updates; the UI thread reads what changed with [`read_since`](Self::read_since),
/// which holds the lock only to copy the new lines.
pub struct UserCommandOutputSession {
    pub request: UserCommandRequestId,
    pub command: UserCommandId,
    pub name: String,
    state: Mutex<SessionState>,
    /// Changed since the last notification.
    changed: AtomicBool,
    /// A notification was sent and the UI has not read since.
    notified: AtomicBool,
    stop: AtomicBool,
}

impl fmt::Debug for UserCommandOutputSession {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("UserCommandOutputSession")
            .field("request", &self.request)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

impl UserCommandOutputSession {
    pub fn new(
        request: UserCommandRequestId,
        command: UserCommandId,
        name: String,
        planned: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            request,
            command,
            name,
            state: Mutex::new(SessionState {
                planned,
                ..SessionState::default()
            }),
            changed: AtomicBool::new(false),
            notified: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        })
    }

    fn state(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn touch(&self) {
        self.changed.store(true, Ordering::Release);
    }

    /// Backend: a process started.
    pub fn begin_run(&self, run: OutputRun) {
        let mut state = self.state();
        let planned = state.planned;
        let notice = if planned > 1 {
            format!("▶ [{}/{}] {}", run.index + 1, planned, run.command_line)
        } else {
            format!("▶ {}", run.command_line)
        };
        state.log.push(OutputLine {
            stream: OutputStream::Notice,
            text: notice,
        });
        state.runs.push(run);
        drop(state);
        self.touch();
    }

    /// Backend: complete lines and the unterminated rest of one stream.
    pub fn append(&self, stream: OutputStream, lines: Vec<String>, partial: Option<String>) {
        let mut state = self.state();
        for text in lines {
            state.log.push(OutputLine { stream, text });
        }
        state.log.set_partial(stream, partial);
        drop(state);
        self.touch();
    }

    /// Backend: the running process ended.
    pub fn end_run(&self, index: u64, exit: Option<ProgramExit>, duration: Duration) {
        let mut state = self.state();
        let stopped = self.stop_requested();
        let mut text = match exit {
            Some(exit) => exit.describe(),
            None => "The exit status is unknown".to_owned(),
        };
        if stopped {
            text = format!("Stopped · {text}");
        }
        text = format!("{text} · {}", describe_duration(duration));
        state.log.push(OutputLine {
            stream: OutputStream::Notice,
            text,
        });
        if let Some(run) = state.runs.iter_mut().rev().find(|run| run.index == index) {
            run.exit = exit;
            run.duration = Some(duration);
            run.stopped = stopped;
        }
        drop(state);
        self.touch();
    }

    /// Backend: no further run starts.
    pub fn finish(&self, cancelled: bool, error: Option<String>) {
        let mut state = self.state();
        if let Some(error) = &error {
            state.log.push(OutputLine {
                stream: OutputStream::Notice,
                text: error.clone(),
            });
        }
        state.finished = Some(OutputFinish { cancelled, error });
        drop(state);
        self.touch();
    }

    /// Backend: whether to tell the UI about new output now. True at most once until
    /// the UI has read ([`read_since`](Self::read_since)), so a busy program does not
    /// flood the event queue.
    pub fn take_notification(&self) -> bool {
        if self.notified.load(Ordering::Acquire) || !self.changed.swap(false, Ordering::AcqRel) {
            return false;
        }
        self.notified.store(true, Ordering::Release);
        true
    }

    /// Backend: the notification `take_notification` allowed could not be sent; the
    /// next one tries again.
    pub fn notification_failed(&self) {
        self.notified.store(false, Ordering::Release);
        self.touch();
    }

    /// UI: what changed after the line `from` (the `next` of its previous view).
    pub fn read_since(&self, from: u64) -> OutputView {
        self.notified.store(false, Ordering::Release);
        let state = self.state();
        OutputView {
            first: state.log.first(),
            lines: state.log.since(from).cloned().collect(),
            next: state.log.next(),
            dropped: state.log.dropped(),
            partial: state.log.partials().collect(),
            runs: state.runs.clone(),
            planned: state.planned,
            finished: state.finished.clone(),
            stopping: self.stop_requested() && state.finished.is_none(),
        }
    }

    /// UI: Clear forgets the shown lines (Copy all and Save as then hold only later ones).
    pub fn clear(&self) {
        self.state().log.clear();
        self.touch();
    }

    /// Everything retained, as plain text (Copy all, Save as).
    pub fn text(&self) -> String {
        self.state().log.text()
    }

    /// Ask the backend to end the running process tree and start no further run.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.touch();
    }

    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub fn is_finished(&self) -> bool {
        self.state().finished.is_some()
    }
}

/// `Opened` asks the UI for a window; later updates only say that it has news.
#[derive(Clone, Debug)]
pub struct UserCommandOutputEvent {
    pub session: Arc<UserCommandOutputSession>,
    pub opened: bool,
}

/// Save as…: the session's retained text, written to `destination`. An existing file
/// is replaced only when `overwrite` carries the user's consent.
#[derive(Clone, Debug)]
pub struct UserCommandOutputSaveRequest {
    pub request: crate::DesktopRequestId,
    pub session: Arc<UserCommandOutputSession>,
    pub destination: std::path::PathBuf,
    pub overwrite: bool,
}

/// A saved output: where, how many bytes, and the replaced file kept for recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserCommandOutputSaveReceipt {
    pub path: std::path::PathBuf,
    pub bytes: u64,
    pub recovery: Vec<std::path::PathBuf>,
}

/// The argument that makes the application executable the Console window launcher:
/// `midnight-explorer --run-and-wait <program> [argument…]` runs the program with exactly those
/// arguments in the console it was started in, waits for it, shows its exit status and
/// keeps the console open until a key is pressed. No shell parses the arguments.
pub const RUN_AND_WAIT_ARGUMENT: &str = "--run-and-wait";

/// The Console window launcher: `executable` started with `leading` arguments, then the
/// program and its arguments. The application sets `leading` to
/// [`RUN_AND_WAIT_ARGUMENT`]; tests use their own executable's fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleLauncher {
    pub executable: std::path::PathBuf,
    pub leading: Vec<OsString>,
}

impl ConsoleLauncher {
    /// The launcher's arguments for `program` and its `arguments`, in order.
    pub fn arguments(&self, program: &Path, arguments: &[OsString]) -> Vec<OsString> {
        let mut all = self.leading.clone();
        all.push(program.as_os_str().to_owned());
        all.extend(arguments.iter().cloned());
        all
    }
}

/// The pipes of a captured program. Reads block; backend reader threads own them.
pub struct CapturedStreams {
    pub stdout: Box<dyn io::Read + Send>,
    pub stderr: Box<dyn io::Read + Send>,
}

/// A Run directly program started with captured output: no window, standard input
/// empty and closed, standard output and error in pipes, and its process tree in one
/// group (Windows job object, Linux process group) that `terminate_tree` ends.
/// Calls and disposal belong to backend workers.
pub trait CapturedUserProgram: Send {
    fn id(&self) -> u32;
    /// The pipes, once.
    fn take_streams(&mut self) -> Option<CapturedStreams>;
    fn try_wait(&mut self) -> io::Result<Option<ProgramExit>>;
    /// End the program and the processes it started that are still in its group.
    /// `force` follows a polite request that was ignored (Linux: SIGKILL after SIGTERM).
    fn terminate_tree(&mut self, force: bool) -> io::Result<()>;
    /// How to decode its lines.
    fn decoder(&self) -> OutputDecoder {
        decode_utf8_lossy
    }
}

#[cfg(test)]
#[path = "user_command_output_tests.rs"]
mod tests;
