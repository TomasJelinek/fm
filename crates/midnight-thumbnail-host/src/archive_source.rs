//! Read-only archive member leases. Display labels never become native operation paths.
use crate::{decode::Source, service::Capture};
use midnight_core::{ArchiveMount, ArchiveNode, EntryKind};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Immutable UI-captured member authority; all validation and reading happens on backend workers.
#[derive(Clone, Debug)]
pub struct ArchiveThumbnailSource {
    pub mount: Arc<dyn ArchiveMount>,
    pub node: ArchiveNode,
    pub source_version: Vec<u8>,
}

impl ArchiveThumbnailSource {
    /// Defer capture until an admitted thumbnail job. Providers must explicitly support stable
    /// member identities before their pixels can enter the persistent thumbnail cache.
    pub fn capture(self) -> Capture {
        Arc::new(move |cancel| {
            check_cancel(cancel)?;
            let length = self
                .node
                .size
                .ok_or_else(|| invalid("Archive member length unavailable"))?;
            if self.node.kind != EntryKind::File
                || !self.node.readable
                || self.node.id.entry < 2
                || self.node.id.mount == 0
                || self.node.id.mount != self.mount.id()
                || self.source_version.is_empty()
                || self.source_version.len() > 256
            {
                return Err(invalid("Invalid archive thumbnail member authority"));
            }
            self.mount.validate().map_err(io::Error::other)?;
            let identity = self
                .mount
                .persistent_member_identity(self.node.id.entry, &self.source_version)
                .map_err(io::Error::other)?
                .filter(|identity| !identity.is_empty() && identity.len() <= 256 * 1024)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Archive provider does not supply a bounded persistent member identity",
                    )
                })?;
            check_cancel(cancel)?;
            // The complete authority is hashed before entering cache record plaintext. Native
            // container names and any future authenticated provider locators remain private.
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"MidnightArchiveThumbnailSource\0v1\0");
            hasher.update(&(identity.len() as u64).to_le_bytes());
            hasher.update(&identity);
            hasher.update(&length.to_le_bytes());
            let source = Member {
                descriptor: self.clone(),
                length,
                version: hasher.finalize().as_bytes().to_vec(),
            };
            source.validate()?;
            check_cancel(cancel)?;
            Ok(Arc::new(source) as Arc<dyn Source>)
        })
    }
}

struct Member {
    descriptor: ArchiveThumbnailSource,
    length: u64,
    version: Vec<u8>,
}
impl Source for Member {
    fn length(&self) -> u64 {
        self.length
    }
    fn version(&self) -> Vec<u8> {
        self.version.clone()
    }
    fn validate(&self) -> io::Result<()> {
        self.descriptor.mount.validate().map_err(io::Error::other)
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.read_at_cooperative(offset, length, &|| false)
    }
    fn read_at_cooperative(
        &self,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Vec<u8>> {
        if length == 0
            || length as usize > midnight_thumbnail_api::MAX_READ_BYTES
            || offset
                .checked_add(u64::from(length))
                .is_none_or(|end| end > self.length)
        {
            return Err(invalid("Archive thumbnail read outside captured content"));
        }
        self.validate()?;
        // Negotiated hosts cancel one member preparation at staging checkpoints.
        // Legacy mounts drain under an independent token. Neither releases the real
        // decoder/I/O capacity before this call and native cleanup have returned.
        let bytes = self
            .descriptor
            .mount
            .read_cooperative(self.descriptor.node.id.entry, offset, length, cancelled)
            .map_err(|error| io::Error::new(error.kind, error))?;
        self.validate()?;
        if bytes.len() != length as usize {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Archive member returned a short read",
            ));
        }
        Ok(bytes)
    }
}
fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
