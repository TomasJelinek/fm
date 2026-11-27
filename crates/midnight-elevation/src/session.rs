//! The requester's side of an authorization session: one consent starts the helper, which
//! lists the authorized folder and then, without another prompt, that folder or folders
//! inside it, until the session ends. This side ends it a little before the helper would
//! (`MARGIN`), so a request never races the helper's own idle or lifetime limit.
use crate::{
    HelperDigest, cancelled, helper, native,
    protocol::{self, Failure, FailureKind, Request, WireEntry},
    trusted_helper,
};
use midnight_core::{
    Cancellation, ElevatedListingSession, Entry, EntryId, EntryKind, FsError, SessionListingError,
};
use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::{Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant, UNIX_EPOCH},
};

/// The operation of listing failures.
const OPERATION: &str = "authorized directory listing";
/// How much earlier than the helper this side considers an idle or old session ended.
const MARGIN: Duration = Duration::from_secs(15);

/// Ask for authorization, list `path` and keep the session if the helper offers one.
pub(crate) fn open(
    path: &Path,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(Entry) -> bool,
    shipped: Option<&HelperDigest>,
) -> io::Result<Option<Box<dyn ElevatedListingSession>>> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    if !path.is_absolute() {
        return Err(protocol::invalid("Directory path must be absolute"));
    }
    let helper = trusted_helper(shipped)?;
    // Measured before the consent prompt: the helper's lifetime starts later.
    let started = Instant::now();
    let request = Request::List(native::encode_path(path));
    let (connection, reply) = native::start(&helper, &request, cancel, shipped)?;
    let entries = reply.result.map_err(failure_error)?;
    emit_entries(path, entries, cancel, emit)?;
    if !reply.session {
        return Ok(None);
    }
    Ok(Some(Box::new(AuthorizedSession::new(
        path,
        connection,
        started + protocol::LIFETIME.saturating_sub(MARGIN),
        protocol::IDLE_TIMEOUT.saturating_sub(MARGIN),
    ))))
}

/// A session over an already connected helper, for in-process tests of both ends.
#[cfg(test)]
pub(crate) fn connected(
    root: &Path,
    connection: native::Connection,
    idle: Duration,
) -> Box<dyn ElevatedListingSession> {
    Box::new(AuthorizedSession::new(
        root,
        connection,
        Instant::now() + protocol::LIFETIME,
        idle,
    ))
}

fn failure_error(failure: Failure) -> io::Error {
    io::Error::new(
        match failure.kind {
            FailureKind::Denied | FailureKind::Outside => io::ErrorKind::PermissionDenied,
            FailureKind::Other => io::ErrorKind::Other,
        },
        failure.message,
    )
}

/// Emit the helper's entries of `path`, refusing anything but a plain name.
fn emit_entries(
    path: &Path,
    entries: Vec<WireEntry>,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(Entry) -> bool,
) -> io::Result<()> {
    for (index, wire) in entries.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let name = native::decode_path(&wire.name)?;
        let mut components = name.components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || name.as_os_str().is_empty()
            || name.file_name() != Some(name.as_os_str())
        {
            return Err(protocol::invalid("Helper returned a non-basename entry"));
        }
        let kind = match wire.kind {
            0 => EntryKind::Directory,
            1 => EntryKind::DirectoryLink,
            2 => EntryKind::File,
            3 => EntryKind::Symlink,
            4 => EntryKind::Other,
            _ => return Err(protocol::invalid("Helper returned an unknown entry kind")),
        };
        let mut entry = Entry::new(EntryId(index as u64 + 1), path.join(name), kind);
        entry.kind = kind;
        entry.metadata.size = wire.size;
        entry.metadata.modified = wire.modified.and_then(|(seconds, nanos)| {
            if seconds >= 0 {
                UNIX_EPOCH.checked_add(Duration::new(seconds as u64, nanos))
            } else {
                UNIX_EPOCH
                    .checked_sub(Duration::from_secs(seconds.unsigned_abs()))
                    .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanos))))
            }
        });
        entry.metadata.native_attributes = wire.native_attributes;
        entry.metadata.unix_mode = wire.unix_mode;
        if !emit(entry) {
            return Err(cancelled());
        }
    }
    Ok(())
}

struct State {
    /// `None` once the session ended.
    connection: Option<native::Connection>,
    idle_until: Instant,
}

/// A live session; dropping it closes the helper's channel, which ends the helper.
struct AuthorizedSession {
    root: PathBuf,
    expires: Instant,
    /// How long after a reply this side considers the session idle.
    idle: Duration,
    state: Mutex<State>,
}

impl AuthorizedSession {
    fn new(root: &Path, connection: native::Connection, expires: Instant, idle: Duration) -> Self {
        Self {
            root: root.to_owned(),
            expires,
            idle,
            state: Mutex::new(State {
                connection: Some(connection),
                idle_until: Instant::now() + idle,
            }),
        }
    }

    /// The state, with an idle or expired session ended.
    fn state(&self) -> MutexGuard<'_, State> {
        self.expire(self.state.lock().unwrap_or_else(|error| error.into_inner()))
    }

    fn expire<'a>(&self, mut state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        let now = Instant::now();
        if now >= self.expires || now >= state.idle_until {
            state.connection = None;
        }
        state
    }
}

impl ElevatedListingSession for AuthorizedSession {
    fn root(&self) -> &Path {
        &self.root
    }

    /// Never waits: a session that is listing right now is live.
    fn is_live(&self) -> bool {
        let state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => return true,
        };
        self.expire(state).connection.is_some()
    }

    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), SessionListingError> {
        let failed = |error| SessionListingError::Failed(FsError::from_io(OPERATION, path, error));
        if cancel.is_cancelled() {
            return Err(failed(cancelled()));
        }
        // The helper decides again on the folder it opens; this spares it a request.
        if !helper::within_by_name(&self.root, path) {
            return Err(SessionListingError::Outside(
                FsError::outside_authorization(path, &self.root, ""),
            ));
        }
        let mut state = self.state();
        let Some(connection) = state.connection.as_mut() else {
            return Err(SessionListingError::Ended);
        };
        let request = Request::List(native::encode_path(path));
        let Ok(reply) = connection.exchange(&request, cancel) else {
            // The channel is out of step or gone (also after a cancellation whose reply did
            // not arrive within the grace): the session ends.
            state.connection = None;
            return Err(SessionListingError::Ended);
        };
        if reply.session {
            state.idle_until = Instant::now() + self.idle;
        } else {
            state.connection = None;
        }
        drop(state);
        match reply.result {
            Ok(entries) => emit_entries(path, entries, cancel, emit).map_err(failed),
            Err(failure) if failure.kind == FailureKind::Outside => {
                Err(SessionListingError::Outside(
                    FsError::outside_authorization(path, &self.root, &failure.message),
                ))
            }
            Err(failure) => Err(failed(failure_error(failure))),
        }
    }
}
