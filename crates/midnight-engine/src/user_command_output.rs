//! Output window runs on backend threads: one reader per stream drains its pipe as fast
//! as the program writes (the program never waits for the UI), and a supervisor per
//! process waits for its exit, answers Stop and tells the UI about new output at most
//! every `OUTPUT_PUBLISH_INTERVAL`.
use midnight_core::*;
use std::{
    io::{self, Read},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

/// After Stop's polite request (Linux SIGTERM), how long before the tree is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// After the program exited, how long its pipes may stay open (a background process it
/// started still holds them) before the run is reported as ended anyway.
const DRAIN_GRACE: Duration = Duration::from_secs(1);
const READ_BUFFER: usize = 64 * 1024;

/// Tells the UI that `session` has news; false when the event could not be queued.
pub(crate) type Publish = Arc<dyn Fn(&Arc<UserCommandOutputSession>) -> bool + Send + Sync>;

/// Drain one stream into the session: terminal control sequences removed, complete lines
/// decoded, the unterminated rest shown as it grows. Ends at the end of the stream.
fn read_stream(
    mut pipe: Box<dyn Read + Send>,
    stream: OutputStream,
    decoder: OutputDecoder,
    session: &UserCommandOutputSession,
) {
    let mut terminal = TerminalLines::default();
    let mut buffer = vec![0u8; READ_BUFFER];
    loop {
        let read = match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let mut lines = Vec::new();
        terminal.feed(&buffer[..read], |line| {
            lines.push(expand_tabs(decoder(&line)));
        });
        let partial = decode_partial(terminal.partial(), decoder);
        session.append(stream, lines, Some(partial));
    }
    let last = terminal.finish().map(|line| expand_tabs(decoder(&line)));
    session.append(stream, last.into_iter().collect(), None);
}

/// Start a reader thread per stream; each sends on `done` when its stream ended. Returns
/// how many started, and why one could not.
fn start_readers(
    program: &mut dyn CapturedUserProgram,
    session: &Arc<UserCommandOutputSession>,
    done: &mpsc::Sender<()>,
) -> (usize, Option<io::Error>) {
    let Some(streams) = program.take_streams() else {
        return (0, None);
    };
    let decoder = program.decoder();
    let mut started = 0;
    for (pipe, stream) in [
        (streams.stdout, OutputStream::Stdout),
        (streams.stderr, OutputStream::Stderr),
    ] {
        let session = session.clone();
        let done = done.clone();
        if let Err(error) = thread::Builder::new()
            .name("fm-command-output".into())
            .spawn(move || {
                read_stream(pipe, stream, decoder, &session);
                let _ = done.send(());
            })
        {
            return (started, Some(error));
        }
        started += 1;
    }
    (started, None)
}

/// Run `program` to its end on this (blocking) thread: drain its output into `session`,
/// end its process tree when `cancel` or the session's Stop asks, and record how it
/// ended as run `index`. Returns its exit, None when the system did not report one.
pub(crate) fn supervise(
    mut program: Box<dyn CapturedUserProgram>,
    session: &Arc<UserCommandOutputSession>,
    index: u64,
    cancel: &Cancellation,
    publish: &Publish,
) -> Option<ProgramExit> {
    let started = Instant::now();
    let (done, ended) = mpsc::channel();
    let (readers, failure) = start_readers(program.as_mut(), session, &done);
    if let Some(error) = failure {
        // Without a reader the program would block on a full pipe: end it.
        let _ = program.terminate_tree(true);
        session.append(
            OutputStream::Notice,
            vec![format!("The output could not be read: {error}")],
            None,
        );
    }
    drop(done);
    let mut open = readers;
    let mut stopped_at: Option<Instant> = None;
    let mut forced = false;
    let mut failures = 0u32;
    let exit = loop {
        if (cancel.is_cancelled() || session.stop_requested()) && stopped_at.is_none() {
            let _ = program.terminate_tree(false);
            stopped_at = Some(Instant::now());
        }
        if !forced && stopped_at.is_some_and(|at| at.elapsed() >= STOP_GRACE) {
            let _ = program.terminate_tree(true);
            forced = true;
        }
        match program.try_wait() {
            Ok(Some(exit)) => break Some(exit),
            Ok(None) => {}
            // A wait that keeps failing proves nothing about the exit; give up on it
            // after a while rather than spinning forever.
            Err(_) => {
                failures += 1;
                if failures > 100 {
                    let _ = program.terminate_tree(true);
                    break None;
                }
            }
        }
        notify(session, publish);
        if open > 0 {
            match ended.recv_timeout(OUTPUT_PUBLISH_INTERVAL) {
                Ok(()) => open -= 1,
                Err(mpsc::RecvTimeoutError::Disconnected) => open = 0,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        } else {
            thread::sleep(OUTPUT_PUBLISH_INTERVAL);
        }
    };
    // The last output arrives after the exit; a background process may keep the pipes.
    let until = Instant::now() + DRAIN_GRACE;
    while open > 0 {
        match ended.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(()) => open -= 1,
            Err(_) => break,
        }
    }
    session.end_run(index, exit, started.elapsed());
    drop(program);
    notify(session, publish);
    exit
}

/// What a supervision needs besides the program, and what it keeps alive while the
/// program runs (`pins`: the process permit and the native services).
pub(crate) struct Supervision {
    pub(crate) session: Arc<UserCommandOutputSession>,
    pub(crate) index: u64,
    pub(crate) cancel: Cancellation,
    pub(crate) publish: Publish,
    pub(crate) _pins: Box<dyn Send>,
}

/// `supervise` on a thread of its own, not the runtime's small blocking pool, which a
/// long-running program would occupy. Falls back to that pool when no thread starts.
pub(crate) async fn supervise_on_thread(
    program: Box<dyn CapturedUserProgram>,
    supervision: Supervision,
) -> Result<Option<ProgramExit>, ()> {
    let (give, take) = mpsc::channel::<(Box<dyn CapturedUserProgram>, Supervision)>();
    let (done, finished) = tokio::sync::oneshot::channel();
    let started = thread::Builder::new()
        .name("fm-command-run".into())
        .spawn(move || {
            if let Ok((program, supervision)) = take.recv() {
                let exit = run(program, &supervision);
                drop(supervision);
                let _ = done.send(exit);
            }
        });
    if started.is_ok() {
        give.send((program, supervision)).map_err(|_| ())?;
        return finished.await.map_err(|_| ());
    }
    tokio::task::spawn_blocking(move || {
        let exit = run(program, &supervision);
        drop(supervision);
        exit
    })
    .await
    .map_err(|_| ())
}

fn run(program: Box<dyn CapturedUserProgram>, supervision: &Supervision) -> Option<ProgramExit> {
    supervise(
        program,
        &supervision.session,
        supervision.index,
        &supervision.cancel,
        &supervision.publish,
    )
}

/// Publish news if there is any and the UI has read the previous news.
pub(crate) fn notify(session: &Arc<UserCommandOutputSession>, publish: &Publish) {
    if session.take_notification() && !publish(session) {
        session.notification_failed();
    }
}

#[cfg(test)]
#[path = "user_command_output_tests.rs"]
mod tests;
