use crate::{
    Cancel, NativeProvider,
    client::{Lease, PendingRelease, check_cancel},
    discovery::invalid,
};
use midnight_filesystem_plugin_api::{Failure, Kind, Request, Response, abi};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

impl NativeProvider {
    /// Capture a regular file through the plugin. No native opener fallback is used.
    pub fn capture(&self, path: &Path, cancelled: Cancel<'_>) -> Result<CapturedFile, Failure> {
        let source = self.capture_source(path, cancelled)?;
        match CapturedFile::from_source(source.clone()) {
            Ok(captured) => Ok(captured),
            Err(error) => Err(release_error(source, invalid(error), &mut false)),
        }
    }

    /// Retain the native lease without adding another public read-validation wrapper.
    pub(crate) fn capture_source(
        &self,
        path: &Path,
        cancelled: Cancel<'_>,
    ) -> Result<Arc<dyn ContentSource>, Failure> {
        self.capture_source_status(path, cancelled, &mut false)
    }

    pub(crate) fn capture_source_status(
        &self,
        path: &Path,
        cancelled: Cancel<'_>,
        unconfirmed: &mut bool,
    ) -> Result<Arc<dyn ContentSource>, Failure> {
        self.require(abi::CAP_READ)?;
        let (mount_lease, mount, node) = self.resolve_status(path, cancelled, unconfirmed)?;
        let mount_lease = PendingRelease::new(mount_lease);
        if node.kind != Kind::File {
            return Err(close_error(
                mount_lease.into_inner(),
                invalid("Viewer requires a regular file"),
                unconfirmed,
            ));
        }
        let response = match self.exchange_status(
            Request::OpenRead {
                mount,
                node: node.id,
            },
            cancelled,
            unconfirmed,
        ) {
            Ok(response) => response,
            Err(error) => {
                // A malformed reply may have supplied another reader whose
                // close failed. Keep the enclosing mount armed in that case.
                if *unconfirmed {
                    return Err(error);
                }
                return Err(close_error(mount_lease.into_inner(), error, unconfirmed));
            }
        };
        let Response::Opened {
            reader,
            length,
            revision,
        } = response
        else {
            return Err(close_error(
                mount_lease.into_inner(),
                invalid("Missing content reader"),
                unconfirmed,
            ));
        };
        if reader == 0 {
            return Err(close_error(
                mount_lease.into_inner(),
                invalid("Invalid content reader"),
                unconfirmed,
            ));
        }
        let reader_lease = Lease::new(self, Request::CloseRead { reader });
        let invalid_revision =
            revision.is_empty() || revision.len() > midnight_filesystem_plugin_api::MAX_REVISION;
        let content: Arc<dyn ContentSource> = Arc::new(Content {
            reader_lease,
            _mount_lease: mount_lease.into_inner(),
            reader,
            length,
            revision,
            name: node.display,
            release_unconfirmed: AtomicBool::new(false),
        });
        if let Err(error) = check_cancel(cancelled) {
            return Err(release_error(content, error, unconfirmed));
        }
        if invalid_revision {
            return Err(release_error(
                content,
                invalid("Invalid content revision"),
                unconfirmed,
            ));
        }
        Ok(content)
    }
}

#[derive(Debug)]
struct Content {
    // Field order is intentional: reader closes before the containing mount.
    reader_lease: Lease,
    _mount_lease: Lease,
    reader: u64,
    length: u64,
    revision: Vec<u8>,
    name: String,
    release_unconfirmed: AtomicBool,
}
impl Content {
    fn exchange_content(&self, request: Request) -> Result<Response, Error> {
        if self.release_unconfirmed.load(Ordering::Acquire) {
            return Err(Error("Native content teardown is unconfirmed".into()));
        }
        let mut unconfirmed = false;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.reader_lease
                .provider
                .exchange_status(request, &|| false, &mut unconfirmed)
        }));
        match result {
            Ok(result) => {
                if unconfirmed {
                    self.release_unconfirmed.store(true, Ordering::Release);
                }
                result.map_err(|error| Error(error.to_string()))
            }
            Err(panic) => {
                std::mem::forget(panic);
                self.release_unconfirmed.store(true, Ordering::Release);
                Err(Error(
                    "Native content callback panicked; teardown is unconfirmed".into(),
                ))
            }
        }
    }
}
impl ContentSource for Content {
    fn cleanup_unconfirmed(&self) -> bool {
        self.release_unconfirmed.load(Ordering::Acquire)
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        // Observe prior completed operations before the shared-reference exit.
        // A clean shared release acquires no new native cleanup responsibility;
        // a concurrently tainting operation retains its own reporting owner.
        if self.cleanup_unconfirmed() {
            std::mem::forget(self);
            return Err(Error("Native content teardown is unconfirmed".into()));
        }
        let Some(content) = Arc::into_inner(self) else {
            return Ok(());
        };
        let mut pending = PendingRelease::new(content);
        if pending.release_unconfirmed.load(Ordering::Acquire) {
            return Err(Error("Native content teardown is unconfirmed".into()));
        }
        pending
            .reader_lease
            .close()
            .map_err(|error| Error(error.to_string()))?;
        pending
            ._mount_lease
            .close()
            .map_err(|error| Error(error.to_string()))?;
        drop(pending.into_inner());
        Ok(())
    }
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn version(&self) -> Vec<u8> {
        self.revision.clone()
    }
    fn validate(&self) -> Result<(), Error> {
        self.exchange_content(Request::ValidateRead {
            reader: self.reader,
        })
        .map(|_| ())
    }
    fn read_checked(
        &self,
        offset: u64,
        length: u32,
        expected_length: u64,
        expected_version: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if self.length != expected_length || self.revision != expected_version {
            return Err(Error("Native file changed after capture".into()));
        }
        // The native SDK's ReadAt contract validates this pinned reader and pathname
        // before and after reading (midnight-filesystem-plugin-sdk/src/native/content.rs).
        self.read_at(offset, length)
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if length == 0
            || length > midnight_filesystem_plugin_api::MAX_READ
            || offset.checked_add(u64::from(length)).is_none()
        {
            return Err(Error("Invalid filesystem content bounds".into()));
        }
        let expected = self.length.saturating_sub(offset).min(u64::from(length));
        if expected == 0 {
            self.validate()?;
            return Ok(Vec::new());
        }
        let response = self.exchange_content(Request::ReadAt {
            reader: self.reader,
            offset,
            length,
        })?;
        let Response::Content { bytes, eof } = response else {
            return Err(Error("Missing content response".into()));
        };
        if bytes.len() as u64 != expected || eof != (offset + expected == self.length) {
            return Err(Error(
                "Filesystem content length or EOF differs from captured revision".into(),
            ));
        }
        Ok(bytes)
    }
}

fn close_error(lease: Lease, original: Failure, unconfirmed: &mut bool) -> Failure {
    match lease.release_on_worker() {
        Ok(()) => original,
        Err(cleanup) => {
            *unconfirmed = true;
            invalid(format!(
                "{original}; native content teardown unconfirmed: {cleanup}"
            ))
        }
    }
}
fn release_error(
    source: Arc<dyn ContentSource>,
    original: Failure,
    unconfirmed: &mut bool,
) -> Failure {
    match source.release_on_worker() {
        Ok(()) => original,
        Err(cleanup) => {
            *unconfirmed = true;
            invalid(format!(
                "{original}; native content teardown unconfirmed: {cleanup}"
            ))
        }
    }
}

#[cfg(test)]
#[path = "native_content_retirement_tests.rs"]
mod retirement_tests;
