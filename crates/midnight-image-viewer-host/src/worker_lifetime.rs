//! One armed backend owner for a decoder, its installed-code pin, source and capacity token.
use std::{
    fmt, io,
    panic::{AssertUnwindSafe, catch_unwind},
};

pub(crate) trait Shutdown {
    fn shutdown(&mut self) -> io::Result<()>;
}

pub(crate) trait Release {
    fn release_on_worker(self) -> io::Result<()>;
}

/// Native cleanup proved by an owned image-worker operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupStatus {
    Confirmed,
    ProcessUnconfirmed,
    PinUnconfirmed,
}

#[derive(Debug)]
struct CleanupError {
    status: CleanupStatus,
    original: io::Error,
}
impl fmt::Display for CleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} [image cleanup: {:?}]",
            self.original, self.status
        )
    }
}
impl std::error::Error for CleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.original)
    }
}

fn worst(left: CleanupStatus, right: CleanupStatus) -> CleanupStatus {
    match (left, right) {
        (CleanupStatus::ProcessUnconfirmed, _) | (_, CleanupStatus::ProcessUnconfirmed) => {
            CleanupStatus::ProcessUnconfirmed
        }
        (CleanupStatus::PinUnconfirmed, _) | (_, CleanupStatus::PinUnconfirmed) => {
            CleanupStatus::PinUnconfirmed
        }
        _ => CleanupStatus::Confirmed,
    }
}

/// Inspect the host's typed outcome before converting an error to text.
pub fn cleanup_status(error: &io::Error) -> Option<CleanupStatus> {
    error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<CleanupError>())
        .map(|cause| cause.status)
}

pub(crate) fn classified(status: CleanupStatus, original: io::Error) -> io::Error {
    let status = worst(
        status,
        cleanup_status(&original).unwrap_or(CleanupStatus::Confirmed),
    );
    io::Error::new(original.kind(), CleanupError { status, original })
}

pub(crate) fn attempt<T>(operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(payload) => {
            // An opaque panic payload may itself own a panicking or native destructor.
            std::mem::forget(payload);
            Err(io::Error::other(
                "Image worker owner panicked during cleanup",
            ))
        }
    }
}

#[derive(Clone, Copy)]
enum State {
    NoProcess,
    Running,
    ProcessUnconfirmed,
    PinUnconfirmed,
    Closed,
}

pub(crate) struct Owner<T: Shutdown, I: Release, S> {
    transport: Option<T>,
    identity: Option<I>,
    source: Option<S>,
    retention: Option<Box<dyn Send>>,
    state: State,
}

impl<T: Shutdown, I: Release> Owner<T, I, ()> {
    pub(crate) fn new(identity: I) -> Self {
        let mut owner = Self::empty(None);
        owner.identity = Some(identity);
        owner
    }

    pub(crate) fn empty(retention: Option<Box<dyn Send>>) -> Self {
        Self {
            transport: None,
            identity: None,
            source: None,
            retention,
            state: State::NoProcess,
        }
    }

    pub(crate) fn with_source<S>(mut self, source: S) -> Owner<T, I, S> {
        let owner = Owner {
            transport: self.transport.take(),
            identity: self.identity.take(),
            source: Some(source),
            retention: self.retention.take(),
            state: self.state,
        };
        self.state = State::Closed;
        owner
    }

    pub(crate) fn into_identity(mut self) -> I {
        assert!(matches!(self.state, State::NoProcess));
        self.identity.take().expect("captured installed-code pin")
    }
}

impl<T: Shutdown, I: Release, S> Owner<T, I, S> {
    #[cfg(test)]
    pub(crate) fn retain(&mut self, owner: Box<dyn Send>) {
        assert!(
            self.retention.is_none(),
            "one reservation per native worker"
        );
        self.retention = Some(owner);
    }

    pub(crate) fn install_identity(&mut self, identity: I) {
        assert!(self.identity.is_none() && matches!(self.state, State::NoProcess));
        self.identity = Some(identity);
    }

    pub(crate) fn start(&mut self, spawn: impl FnOnce() -> io::Result<T>) -> io::Result<()> {
        assert!(self.transport.is_none() && matches!(self.state, State::NoProcess));
        // An opaque spawn failure can follow process creation without returning its handle.
        self.state = State::ProcessUnconfirmed;
        let transport =
            attempt(spawn).map_err(|error| classified(CleanupStatus::ProcessUnconfirmed, error))?;
        self.transport = Some(transport);
        self.state = State::Running;
        Ok(())
    }

    pub(crate) fn identity(&self) -> &I {
        self.identity.as_ref().expect("live installed-code owner")
    }

    pub(crate) fn source(&self) -> &S {
        self.source.as_ref().expect("attached source owner")
    }

    pub(crate) fn source_mut(&mut self) -> &mut S {
        self.source.as_mut().expect("attached source owner")
    }

    pub(crate) fn transport_mut(&mut self) -> &mut T {
        self.transport.as_mut().expect("started transport")
    }

    pub(crate) fn parts(&mut self) -> (&mut T, &I, Option<&S>) {
        (
            self.transport.as_mut().expect("started transport"),
            self.identity.as_ref().expect("live installed-code owner"),
            self.source.as_ref(),
        )
    }

    pub(crate) fn finish(&mut self) -> io::Result<()> {
        let unknown = match self.state {
            State::ProcessUnconfirmed => Some(CleanupStatus::ProcessUnconfirmed),
            State::PinUnconfirmed => Some(CleanupStatus::PinUnconfirmed),
            State::Closed => return Ok(()),
            State::NoProcess | State::Running => None,
        };
        if let Some(status) = unknown {
            return Err(classified(
                status,
                io::Error::other("Previous cleanup was unconfirmed"),
            ));
        }
        // Arm every tail before any native call, destructor or consuming pin release.
        self.state = State::ProcessUnconfirmed;
        let result = attempt(|| {
            if let Some(transport) = &mut self.transport {
                transport.shutdown()?;
            }
            drop(self.transport.take());
            // The child is positively gone. Source and capacity no longer protect a live process.
            self.state = State::PinUnconfirmed;
            drop(self.source.take());
            drop(self.retention.take());
            if let Some(identity) = self.identity.take() {
                identity.release_on_worker()?;
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                self.state = State::Closed;
                Ok(())
            }
            Err(error) => Err(classified(
                match self.state {
                    State::PinUnconfirmed => CleanupStatus::PinUnconfirmed,
                    _ => CleanupStatus::ProcessUnconfirmed,
                },
                error,
            )),
        }
    }

    pub(crate) fn fail(&mut self, original: io::Error) -> io::Error {
        match self.finish() {
            Ok(()) => classified(CleanupStatus::Confirmed, original),
            Err(cleanup) => classified(
                worst(
                    cleanup_status(&original).unwrap_or(CleanupStatus::Confirmed),
                    cleanup_status(&cleanup).unwrap_or(CleanupStatus::ProcessUnconfirmed),
                ),
                io::Error::new(original.kind(), format!("{original}; {cleanup}")),
            ),
        }
    }
}

impl<T: Shutdown, I: Release, S> Drop for Owner<T, I, S> {
    fn drop(&mut self) {
        if self.finish().is_err() {
            // No second native attempt, no pin Drop, and no capacity release after uncertainty.
            std::mem::forget(self.transport.take());
            std::mem::forget(self.identity.take());
            std::mem::forget(self.source.take());
            std::mem::forget(self.retention.take());
        }
    }
}

#[cfg(test)]
#[path = "worker_lifetime_tests.rs"]
mod tests;
