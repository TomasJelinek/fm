//! A pane's live authorization ([`ElevatedListingSession`]). After **Retry with
//! authorization** the pane keeps the session its retry opened: a listing of the authorized
//! folder or a folder inside it that this process's own rights are denied is read through
//! the session, without another prompt. Any other dispatch for the pane (a folder outside it,
//! an archive or network view, another retry) ends the session, and so does shutdown. Ending
//! a session drops it, which closes its helper.
use midnight_core::{
    Cancellation, Command, ElevatedDirectoryService, ElevatedListingSession, Entry, FileSystem,
    FsError, NavigationId, PaneId, SessionListingError,
};
use std::{
    cell::Cell,
    path::{Component, Path},
    sync::{Arc, Mutex, MutexGuard},
};

pub(super) type Session = Arc<dyn ElevatedListingSession>;

#[derive(Default)]
struct Slot {
    /// The pane's newest dispatched navigation.
    current: Option<NavigationId>,
    session: Option<Session>,
}

/// Each pane's session, shared by the command loop and its listing workers.
#[derive(Clone, Default)]
pub(super) struct AuthorizedSessions(Arc<Mutex<[Slot; 2]>>);

/// Whether `path` names `root` or a folder below it by ordinary names, as the helper decides
/// by name before it opens anything.
fn inside(root: &Path, path: &Path) -> bool {
    let mut names = path.components();
    root.components().all(|name| names.next() == Some(name))
        && names.all(|name| matches!(name, Component::Normal(_)))
}

impl AuthorizedSessions {
    fn slots(&self) -> MutexGuard<'_, [Slot; 2]> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The backend dispatched `navigation` for `pane`. The pane keeps a live session only
    /// while the command lists its folder or a folder inside it (a Navigate, Refresh or
    /// Resort, not an authorized `retry`); otherwise the session ends and is returned, for
    /// the caller to drop.
    pub(super) fn dispatched(
        &self,
        pane: PaneId,
        navigation: NavigationId,
        command: Option<&Command>,
        retry: bool,
    ) -> Option<Session> {
        let path = match command {
            Some(
                Command::Navigate { path, .. }
                | Command::Refresh { path, .. }
                | Command::Resort { path, .. },
            ) if !retry => Some(path.as_path()),
            _ => None,
        };
        let mut slots = self.slots();
        let slot = &mut slots[pane.index()];
        slot.current = Some(navigation);
        let keep = slot.session.as_ref().is_some_and(|session| {
            path.is_some_and(|path| inside(session.root(), path)) && session.is_live()
        });
        if keep { None } else { slot.session.take() }
    }

    /// The session that may list `path` for `navigation`, while that is the pane's newest
    /// dispatch.
    fn route(&self, pane: PaneId, navigation: NavigationId, path: &Path) -> Option<Session> {
        let slots = self.slots();
        let slot = &slots[pane.index()];
        slot.session
            .clone()
            .filter(|session| slot.current == Some(navigation) && inside(session.root(), path))
    }

    /// Keep the session that the authorized retry `navigation` opened, unless a newer
    /// dispatch superseded it. Returns what the caller drops: that session, or a replaced one.
    fn install(&self, pane: PaneId, navigation: NavigationId, session: Session) -> Option<Session> {
        let mut slots = self.slots();
        let slot = &mut slots[pane.index()];
        if slot.current != Some(navigation) {
            return Some(session);
        }
        slot.session.replace(session)
    }

    /// `session` reported that it has ended.
    fn ended(&self, pane: PaneId, session: &Session) -> Option<Session> {
        let mut slots = self.slots();
        let slot = &mut slots[pane.index()];
        slot.session
            .take_if(|current| Arc::ptr_eq(current, session))
    }

    /// Every session, for the caller to drop at shutdown.
    pub(super) fn end_all(&self) -> Vec<Session> {
        self.slots()
            .iter_mut()
            .filter_map(|slot| slot.session.take())
            .collect()
    }
}

/// How a pane's native listing reads its folder.
pub(super) struct Reader<'a> {
    /// The authorized retry's service, which opens a session.
    pub elevated: Option<&'a dyn ElevatedDirectoryService>,
    pub filesystem: &'a dyn FileSystem,
    pub sessions: &'a AuthorizedSessions,
    pub pane: PaneId,
    pub navigation: NavigationId,
    /// Set while, and once, the rows come from the authorization helper: the retry's, or the
    /// pane's session's. This process may not read the folder itself.
    pub authorized: &'a Cell<bool>,
}

impl Reader<'_> {
    /// Read `path`. An authorized retry asks for authorization and keeps its session for the
    /// pane. Otherwise the folder is read with this process's rights and, when that is denied
    /// before anything was listed and the pane's session covers `path`, through the session:
    /// a session that ended reports the original denial, which offers a retry as before; a
    /// folder the helper finds outside the authorized one is a denial of its own.
    pub(super) fn read(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if let Some(service) = self.elevated {
            self.authorized.set(true);
            let session = service.open_session(path, cancel, emit)?;
            if let Some(session) = session {
                drop(
                    self.sessions
                        .install(self.pane, self.navigation, Arc::from(session)),
                );
            }
            return Ok(());
        }
        let mut emitted = false;
        let denied = match self.filesystem.read_directory(path, cancel, &mut |entry| {
            emitted = true;
            emit(entry)
        }) {
            Err(error) if error.kind == std::io::ErrorKind::PermissionDenied && !emitted => error,
            other => return other,
        };
        let Some(session) = self.sessions.route(self.pane, self.navigation, path) else {
            return Err(denied);
        };
        self.authorized.set(true);
        match session.read_directory(path, cancel, emit) {
            Ok(()) => Ok(()),
            Err(SessionListingError::Failed(error)) => Err(error),
            Err(SessionListingError::Outside(error)) => {
                self.authorized.set(false);
                Err(error)
            }
            Err(SessionListingError::Ended) => {
                self.authorized.set(false);
                drop(self.sessions.ended(self.pane, &session));
                Err(denied)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ordinary_names_below_the_authorized_folder_are_inside_it() {
        let root = std::env::temp_dir().join("authorized");
        assert!(inside(&root, &root));
        assert!(inside(&root, &root.join("a").join("b")));
        for outside in [
            root.parent().unwrap().to_owned(),
            root.join(".."),
            root.join("a").join("..").join(".."),
            root.with_file_name("authorized-sibling"),
            root.with_file_name("authorize"),
        ] {
            assert!(!inside(&root, &outside), "{}", outside.display());
        }
    }
}
