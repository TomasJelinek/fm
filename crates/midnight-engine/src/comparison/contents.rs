//! Compare contents: a bounded pool of readers that read file pairs byte by byte
//! while the folder walk goes on. The queue holds at most `QUEUE` pairs, each
//! reader two `READ_BYTES` buffers; nothing is written. Every file is opened
//! through the native search service, which pins it without following links, and
//! must still have the size (and time) it was listed with; it is revalidated after
//! the read, so a file written meanwhile is reported as not verified.
use super::compute::{compare_error, interrupted};
use midnight_core::*;
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::Scope,
    time::{Duration, SystemTime},
};

/// Readers working at once: enough to overlap the latency of two files' opens and
/// reads, few enough not to thrash one disk.
pub(super) const READERS: usize = 3;
/// Pairs waiting for a reader.
const QUEUE: usize = 64;
const READ_BYTES: usize = 128 * 1024;
const POLL: Duration = Duration::from_millis(25);

/// Where a pair's outcome goes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Origin {
    /// Displayed rows, left then right.
    Row([usize; 2]),
    /// A folder pair node of the walk.
    Node(u32),
}

/// What a listed file must still be when it is opened.
#[derive(Clone, Copy, Debug)]
pub(super) struct Expected {
    pub(super) size: u64,
    /// None for displayed rows, which are checked against their size only.
    pub(super) modified: Option<SystemTime>,
}

pub(super) struct Candidate {
    pub(super) origin: Origin,
    /// Its key in the comparison's marks, when it has one.
    pub(super) key: Option<OsString>,
    pub(super) paths: [PathBuf; 2],
    pub(super) expected: [Expected; 2],
}

pub(super) struct Outcome {
    pub(super) origin: Origin,
    pub(super) key: Option<OsString>,
    pub(super) path: PathBuf,
    /// Equal bytes (None), the offset of the first differing byte, or why the pair
    /// could not be read.
    pub(super) result: Result<Option<u64>, Failure>,
}

/// Why a pair could not be read: the error, and for the marks whether a file
/// changed meanwhile (`CHANGED`) or could not be read (`UNREADABLE`).
pub(super) struct Failure {
    pub(super) error: FsError,
    pub(super) why: CompareFlags,
}

impl Failure {
    fn unreadable(error: FsError) -> Self {
        Self {
            error,
            why: CompareFlags::UNREADABLE,
        }
    }

    fn changed(error: FsError) -> Self {
        Self {
            error,
            why: CompareFlags::CHANGED,
        }
    }
}

/// What the readers report for progress.
pub(super) struct Shared {
    pub(super) bytes: u64,
    pub(super) current: Option<Arc<[PathBuf; 2]>>,
}

#[derive(Default)]
struct State {
    bytes: AtomicU64,
    current: Mutex<Option<Arc<[PathBuf; 2]>>>,
    stop: AtomicBool,
    closed: AtomicBool,
}

pub(super) struct Pool {
    sender: Mutex<Option<SyncSender<Candidate>>>,
    outcomes: Receiver<Outcome>,
    /// Answers for pairs no reader could take.
    orphans: Mutex<Vec<Outcome>>,
    state: Arc<State>,
}

impl Pool {
    /// Starts `readers` readers in `scope`; none when nothing will be read.
    pub(super) fn start<'scope>(
        scope: &'scope Scope<'scope, '_>,
        find: Option<&'scope dyn FindService>,
        cancel: &'scope Cancellation,
        readers: usize,
    ) -> Self {
        let (sender, queue) = mpsc::sync_channel::<Candidate>(QUEUE);
        let (answer, outcomes) = mpsc::channel();
        let state = Arc::new(State::default());
        let queue = Arc::new(Mutex::new(queue));
        if let Some(find) = find {
            for _ in 0..readers {
                let queue = queue.clone();
                let answer = answer.clone();
                let state = state.clone();
                scope.spawn(move || read_pairs(find, cancel, &queue, &answer, &state));
            }
        }
        Self {
            sender: Mutex::new(Some(sender)),
            outcomes,
            orphans: Mutex::new(Vec::new()),
            state,
        }
    }

    /// Queues `candidate`, or gives it back when the queue is full. A pool without
    /// readers answers at once that it cannot read.
    pub(super) fn offer(&self, candidate: Candidate) -> Option<Candidate> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let candidate = match sender.as_ref() {
            Some(queue) => match queue.try_send(candidate) {
                Ok(()) => return None,
                Err(TrySendError::Full(candidate)) => return Some(candidate),
                Err(TrySendError::Disconnected(candidate)) => candidate,
            },
            None => candidate,
        };
        drop(sender);
        // No reader is left: answer for it, as not verified.
        let path = candidate.paths[0].clone();
        let result = Err(Failure::unreadable(compare_error(
            &path,
            "No content reader is available.",
        )));
        self.orphans
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(Outcome {
                origin: candidate.origin,
                key: candidate.key,
                path,
                result,
            });
        None
    }

    /// No more pairs follow: readers end once the queue is empty.
    pub(super) fn finish(&self) {
        self.sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }

    /// One answered pair; `wait` waits briefly for one.
    pub(super) fn outcome(&self, wait: bool) -> Option<Outcome> {
        if let Some(orphan) = self
            .orphans
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop()
        {
            return Some(orphan);
        }
        let outcome = if wait {
            self.outcomes
                .recv_timeout(POLL * 4)
                .map_err(|error| error == RecvTimeoutError::Disconnected)
        } else {
            self.outcomes
                .try_recv()
                .map_err(|error| error == mpsc::TryRecvError::Disconnected)
        };
        match outcome {
            Ok(outcome) => Some(outcome),
            Err(closed) => {
                if closed {
                    self.state.closed.store(true, Ordering::SeqCst);
                }
                None
            }
        }
    }

    /// Every reader has ended: no more outcomes will come.
    pub(super) fn closed(&self) -> bool {
        self.state.closed.load(Ordering::SeqCst)
    }

    pub(super) fn progress(&self) -> Shared {
        Shared {
            bytes: self.state.bytes.load(Ordering::Relaxed),
            current: self
                .state
                .current
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        }
    }
}

impl Drop for Pool {
    /// Readers stop at their next chunk, and the scope joins them.
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        self.finish();
    }
}

fn read_pairs(
    find: &dyn FindService,
    cancel: &Cancellation,
    queue: &Mutex<Receiver<Candidate>>,
    answer: &mpsc::Sender<Outcome>,
    state: &State,
) {
    let mut buffers = [vec![0; READ_BYTES], vec![0; READ_BYTES]];
    loop {
        if state.stop.load(Ordering::SeqCst) || cancel.is_cancelled() {
            return;
        }
        let candidate = {
            let queue = queue.lock().unwrap_or_else(|error| error.into_inner());
            match queue.recv_timeout(POLL) {
                Ok(candidate) => candidate,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        *state
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(candidate.paths.clone()));
        let result = same_content(find, &candidate, cancel, state, &mut buffers);
        let outcome = Outcome {
            origin: candidate.origin,
            key: candidate.key,
            path: candidate.paths[0].clone(),
            result,
        };
        if answer.send(outcome).is_err() {
            return;
        }
    }
}

/// None when both files hold the same bytes, otherwise the offset of the first
/// byte that differs.
fn same_content(
    find: &dyn FindService,
    candidate: &Candidate,
    cancel: &Cancellation,
    state: &State,
    buffers: &mut [Vec<u8>; 2],
) -> Result<Option<u64>, Failure> {
    let open = |side: usize| -> Result<Box<dyn FindSource>, Failure> {
        let path = &candidate.paths[side];
        let expected = candidate.expected[side];
        let source = find
            .open_find_source(path, cancel)
            .map_err(Failure::unreadable)?;
        let entry = source.entry();
        let same = entry.kind == EntryKind::File
            && entry.metadata.size == Some(expected.size)
            && expected
                .modified
                .is_none_or(|modified| entry.metadata.modified == Some(modified));
        if same {
            Ok(source)
        } else {
            Err(Failure::changed(compare_error(
                path,
                "The file changed after it was listed; compare again.",
            )))
        }
    };
    let mut sources = [open(0)?, open(1)?];
    let size = candidate.expected[0].size;
    let mut total = 0u64;
    let difference = loop {
        if state.stop.load(Ordering::SeqCst) {
            return Err(Failure::unreadable(interrupted(&candidate.paths[0])));
        }
        let [left, right] = buffers;
        let [first, second] = &mut sources;
        let a = fill(first.as_mut(), left, cancel).map_err(Failure::unreadable)?;
        let b = fill(second.as_mut(), right, cancel).map_err(Failure::unreadable)?;
        state.bytes.fetch_add((a + b) as u64, Ordering::Relaxed);
        if a != b || left[..a] != right[..b] {
            let common = a.min(b);
            let offset = left[..common]
                .iter()
                .zip(&right[..common])
                .position(|(x, y)| x != y)
                .unwrap_or(common);
            break Some(total + offset as u64);
        }
        if a == 0 {
            break None;
        }
        total += a as u64;
        if total > size {
            return Err(Failure::changed(compare_error(
                &candidate.paths[0],
                "The file grew while it was read; compare again.",
            )));
        }
    };
    if difference.is_none() && total != size {
        return Err(Failure::changed(compare_error(
            &candidate.paths[0],
            "The file shrank while it was read; compare again.",
        )));
    }
    // A file written during the read cannot prove either answer.
    for source in &sources {
        source.revalidate(cancel).map_err(Failure::changed)?;
    }
    Ok(difference)
}

/// Fills `buffer` unless the file ends first; short provider reads are normal.
fn fill(
    source: &mut dyn FindSource,
    buffer: &mut [u8],
    cancel: &Cancellation,
) -> Result<usize, FsError> {
    let mut filled = 0;
    while filled < buffer.len() {
        if cancel.is_cancelled() {
            return Err(interrupted(&source.entry().path));
        }
        let read = source.read_chunk(&mut buffer[filled..], cancel)?;
        if read > buffer.len() - filled {
            return Err(compare_error(
                &source.entry().path,
                "The search service returned an invalid read count.",
            ));
        }
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}
