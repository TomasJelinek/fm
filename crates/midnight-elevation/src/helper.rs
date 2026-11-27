//! The privileged side of an authorization session. The first request names the folder that
//! the user authorized; later requests are served only for that folder or a folder inside it,
//! and nothing else. Containment is decided twice: by name, before anything is opened, and on
//! the opened folder's final location, so a link, junction or mount point inside the folder
//! cannot lead elsewhere. Folders are listed from enumeration metadata through the open
//! handle; no entry is opened and no file is read. The process has no plugin or shell entry
//! point.
use crate::{
    native,
    protocol::{self, Failure, FailureKind, Reply, Request, WireEntry},
};
use midnight_core::Cancellation;
use std::{
    fmt, io,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

/// How long a session waits and lives (`protocol`); tests shorten them.
pub(crate) struct Limits {
    pub request: Duration,
    pub idle: Duration,
    pub lifetime: Duration,
}
impl Limits {
    pub(crate) const SESSION: Self = Self {
        request: protocol::REQUEST_TIMEOUT,
        idle: protocol::IDLE_TIMEOUT,
        lifetime: protocol::LIFETIME,
    };
}

/// The authorized folder: as requested, and where the system resolved it.
struct Root {
    requested: PathBuf,
    location: native::Location,
}

/// One session's state: no folder until the first listing succeeded.
#[derive(Default)]
pub(crate) struct Session {
    root: Option<Root>,
}

/// A request for a folder that is not the authorized folder or inside it.
#[derive(Debug)]
struct Outside;
impl fmt::Display for Outside {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("The folder is not the authorized folder or inside it.")
    }
}
impl std::error::Error for Outside {}
fn outside() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, Outside)
}
/// Whether `path` names `root` or a folder below it through ordinary names only: no `.` or
/// `..`, no other drive, share or prefix. Names are compared exactly, as the requester's
/// routing compares them.
pub(crate) fn within_by_name(root: &Path, path: &Path) -> bool {
    let mut names = path.components();
    for expected in root.components() {
        if names.next() != Some(expected) {
            return false;
        }
    }
    names.all(|name| matches!(name, Component::Normal(_)))
}

impl Session {
    /// Whether the first listing established the session.
    pub(crate) fn established(&self) -> bool {
        self.root.is_some()
    }

    /// List the folder named by the native `path`. The first successful listing makes its
    /// folder the session's root, if the system can tell where that folder is.
    pub(crate) fn list(&mut self, path: &[u8], cancel: &Cancellation) -> Reply {
        let result = self.collect(path, cancel).map_err(|error| Failure {
            kind: if error.get_ref().is_some_and(|inner| inner.is::<Outside>()) {
                FailureKind::Outside
            } else if error.kind() == io::ErrorKind::PermissionDenied {
                FailureKind::Denied
            } else {
                FailureKind::Other
            },
            message: error.to_string().chars().take(2048).collect(),
        });
        Reply {
            session: self.established(),
            result,
        }
    }

    fn collect(&mut self, path: &[u8], cancel: &Cancellation) -> io::Result<Vec<WireEntry>> {
        if cancel.is_cancelled() {
            return Err(crate::cancelled());
        }
        let path = native::decode_path(path)?;
        if !path.is_absolute() {
            return Err(protocol::invalid("Listing path must be absolute"));
        }
        if let Some(root) = &self.root
            && !within_by_name(&root.requested, &path)
        {
            return Err(outside());
        }
        let directory = native::open_directory(&path)?;
        if let Some(root) = &self.root {
            if !root.location.contains(&directory.location()?) {
                return Err(outside());
            }
            return native::list_directory(&directory, cancel);
        }
        let entries = native::list_directory(&directory, cancel)?;
        // Without a known location the first listing stays a single one.
        if let Ok(location) = directory.location() {
            self.root = Some(Root {
                requested: path,
                location,
            });
        }
        Ok(entries)
    }
}

/// The entries of one reply, within the protocol's bounds.
#[derive(Default)]
pub(crate) struct Entries {
    entries: Vec<WireEntry>,
    bytes: usize,
}
impl Entries {
    pub(crate) fn push(&mut self, entry: WireEntry) -> io::Result<()> {
        self.bytes = self.bytes.saturating_add(entry.name.len() + 64);
        if self.entries.len() >= protocol::MAX_ENTRIES || self.bytes > protocol::MAX_FRAME - 1024 {
            return Err(protocol::invalid(
                "Directory exceeds elevated listing limits",
            ));
        }
        self.entries.push(entry);
        Ok(())
    }
    pub(crate) fn into_vec(self) -> Vec<WireEntry> {
        self.entries
    }
}

/// The helper's authenticated channel to its requester.
pub(crate) trait Requester: Read + Write {
    /// Watches the requester while a folder is listed.
    type Watch;
    /// Bound the following reads and writes by `deadline`.
    fn set_deadline(&mut self, deadline: Instant);
    /// End this process if the requester exits, closes the channel or sends anything, or
    /// `deadline` passes, until the returned watch is dropped. Ending the process also ends
    /// a native directory read that is stuck.
    fn watch(&self, deadline: Instant, cancel: &Cancellation) -> io::Result<Self::Watch>;
}

/// Serve the session's requests until the requester closes it or disconnects, it stays idle
/// for `limits.idle`, or `limits.lifetime` since `started` passes. `first` bounds the first
/// exchange; a failed first listing ends the session.
pub(crate) fn serve_session(
    requester: &mut impl Requester,
    started: Instant,
    first: Instant,
    limits: &Limits,
) -> io::Result<()> {
    let end = started + limits.lifetime;
    let mut session = Session::default();
    let cancel = Cancellation::default();
    let mut wait = first.min(end);
    loop {
        requester.set_deadline(wait);
        let path = match protocol::read_request(requester) {
            Ok(Request::List(path)) => path,
            Ok(Request::Close) => return Ok(()),
            // Between requests, a requester that left or an idle or expired session ends it.
            Err(error)
                if session.established()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof | io::ErrorKind::TimedOut
                    ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let deadline = if session.established() {
            (Instant::now() + limits.request).min(end)
        } else {
            wait
        };
        requester.set_deadline(deadline);
        let watch = requester.watch(deadline, &cancel)?;
        let reply = session.list(&path, &cancel);
        // Stop watching before the reply is written: afterwards the requester may close.
        drop(watch);
        protocol::write_reply(requester, &reply)?;
        if !reply.session {
            return Ok(());
        }
        wait = (Instant::now() + limits.idle).min(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_by_name_accepts_the_root_and_ordinary_names_below_it() {
        let root = if cfg!(windows) {
            Path::new(r"C:\Users\other")
        } else {
            Path::new("/home/other")
        };
        assert!(within_by_name(root, root));
        assert!(within_by_name(root, &root.join("child")));
        assert!(within_by_name(root, &root.join("child").join("grandchild")));
        assert!(within_by_name(
            root,
            &root.join("child").join(".").join("x")
        ));
        for refused in [
            root.join(".."),
            root.join("child").join("..").join(".."),
            root.join("..").join("other").join("child"),
            root.parent().unwrap().to_owned(),
            root.with_file_name("other-sibling"),
            root.with_file_name("othe"),
        ] {
            assert!(!within_by_name(root, &refused), "{}", refused.display());
        }
        if cfg!(windows) {
            for refused in [
                r"D:\Users\other\child",
                r"\\server\share\Users\other",
                r"\\?\C:\Users\other\child",
                r"c:\users\other\child",
            ] {
                assert!(!within_by_name(root, Path::new(refused)), "{refused}");
            }
        } else {
            assert!(!within_by_name(root, Path::new("other/child")));
        }
    }
}
