use crate::{
    Cancel, NativeProvider,
    client::{Lease, check_cancel},
    discovery::invalid,
};
use midnight_filesystem_plugin_api::{Failure, Kind, Request, Response, abi};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{path::Path, sync::Arc};

impl NativeProvider {
    /// Capture a regular file through the plugin. No native opener fallback is used.
    pub fn capture(&self, path: &Path, cancelled: Cancel<'_>) -> Result<CapturedFile, Failure> {
        CapturedFile::from_source(self.capture_source(path, cancelled)?).map_err(invalid)
    }

    /// Retain the native lease without adding another public read-validation wrapper.
    pub(crate) fn capture_source(
        &self,
        path: &Path,
        cancelled: Cancel<'_>,
    ) -> Result<Arc<dyn ContentSource>, Failure> {
        self.require(abi::CAP_READ)?;
        let (mount_lease, mount, node) = self.resolve(path, cancelled)?;
        if node.kind != Kind::File {
            return Err(invalid("Viewer requires a regular file"));
        }
        let Response::Opened {
            reader,
            length,
            revision,
        } = self.exchange(
            Request::OpenRead {
                mount,
                node: node.id,
            },
            cancelled,
        )?
        else {
            return Err(invalid("Missing content reader"));
        };
        if reader == 0 {
            return Err(invalid("Invalid content reader"));
        }
        let reader_lease = Lease::new(self, Request::CloseRead { reader });
        check_cancel(cancelled)?;
        if revision.is_empty() || revision.len() > midnight_filesystem_plugin_api::MAX_REVISION {
            return Err(invalid("Invalid content revision"));
        }
        Ok(Arc::new(Content {
            reader_lease,
            _mount_lease: mount_lease,
            reader,
            length,
            revision,
            name: node.display,
        }))
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
}
impl ContentSource for Content {
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
        self.reader_lease
            .provider
            .exchange(
                Request::ValidateRead {
                    reader: self.reader,
                },
                &|| false,
            )
            .map(|_| ())
            .map_err(|error| Error(error.to_string()))
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
        let response = self
            .reader_lease
            .provider
            .exchange(
                Request::ReadAt {
                    reader: self.reader,
                    offset,
                    length,
                },
                &|| false,
            )
            .map_err(|error| Error(error.to_string()))?;
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
