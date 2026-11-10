//! Read-only relative resources supplied by explicitly captured directory capabilities.
use midnight_core::{Cancellation, EntryKind, TransferDirectory, TransferSource};
use midnight_plugin_host::{CapturedFile, ContentResources, ContentSource, Error};
use std::{
    ffi::OsStr,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_READ: u32 = 256 * 1024;
static VERSIONS: AtomicU64 = AtomicU64::new(1);

fn failure(message: &str) -> Error {
    Error(message.into())
}
fn converted(error: midnight_core::FsError) -> Error {
    // FsError's Display includes the native operation path, which is not resource metadata.
    Error(format!("Relative resource unavailable ({:?})", error.kind))
}

pub(crate) fn validate_segments(segments: &[String]) -> Result<(), Error> {
    if segments.is_empty()
        || segments.len() > 256
        || segments
            .iter()
            .fold(0usize, |total, part| total.saturating_add(part.len()))
            > 64 * 1024
        || segments.iter().any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.bytes().any(|byte| b"/\\:\0".contains(&byte))
        })
    {
        return Err(failure("Invalid relative resource components"));
    }
    Ok(())
}

/// Grant assets beneath an already pinned no-follow directory, tied to the main document lease.
pub fn captured_directory_resources(
    directory: Arc<dyn TransferDirectory>,
    main: CapturedFile,
) -> Result<Arc<dyn ContentResources>, Error> {
    main.revalidate()?;
    directory
        .revalidate(&Cancellation::default())
        .map_err(converted)?;
    Ok(Arc::new(Scope { directory, main }))
}

struct Scope {
    directory: Arc<dyn TransferDirectory>,
    main: CapturedFile,
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CapturedDirectoryResources")
    }
}

impl ContentResources for Scope {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, Error> {
        validate_segments(segments)?;
        self.main.revalidate()?;
        let cancel = Cancellation::default();
        self.directory.revalidate(&cancel).map_err(converted)?;
        let mut source = self
            .directory
            .open_read_child(OsStr::new(&segments[0]), &cancel)
            .map_err(converted)?;
        let mut ancestors = Vec::new();
        for segment in &segments[1..] {
            if source.entry().item.kind != EntryKind::Directory {
                return Err(failure(
                    "Relative resource ancestor is not an ordinary directory",
                ));
            }
            let child = source
                .open_child(OsStr::new(segment), &cancel)
                .map_err(converted)?;
            ancestors.push(source);
            source = child;
        }
        if source.entry().item.kind != EntryKind::File || !source.supports_read_retry() {
            return Err(failure(
                "Relative resources require retained seekable regular files",
            ));
        }
        let length = source
            .entry()
            .item
            .metadata
            .size
            .ok_or_else(|| failure("Relative resource length unavailable"))?;
        let revision = VERSIONS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| failure("Relative resource revision IDs exhausted"))?;
        let asset = Asset {
            directory: self.directory.clone(),
            main: self.main.clone(),
            handles: Mutex::new(Handles { ancestors, source }),
            length,
            name: segments[segments.len() - 1].clone(),
            version: revision.to_le_bytes().to_vec(),
        };
        CapturedFile::from_source(Arc::new(asset))
    }
}

struct Handles {
    ancestors: Vec<Box<dyn TransferSource>>,
    source: Box<dyn TransferSource>,
}

impl Handles {
    fn validate(&self, cancel: &Cancellation) -> Result<(), Error> {
        for ancestor in &self.ancestors {
            ancestor.revalidate(cancel).map_err(converted)?;
        }
        self.source.revalidate(cancel).map_err(converted)
    }
}

struct Asset {
    directory: Arc<dyn TransferDirectory>,
    main: CapturedFile,
    handles: Mutex<Handles>,
    length: u64,
    name: String,
    version: Vec<u8>,
}

impl fmt::Debug for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CapturedRelativeResource")
    }
}

impl Asset {
    fn validate_parent(&self, cancel: &Cancellation) -> Result<(), Error> {
        self.main.revalidate()?;
        self.directory.revalidate(cancel).map_err(converted)
    }
}

impl ContentSource for Asset {
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn version(&self) -> Vec<u8> {
        self.version.clone()
    }
    fn validate(&self) -> Result<(), Error> {
        let cancel = Cancellation::default();
        self.validate_parent(&cancel)?;
        self.handles
            .lock()
            .map_err(|_| failure("Relative resource lock failed"))?
            .validate(&cancel)
    }
    fn read_checked(
        &self,
        offset: u64,
        length: u32,
        expected_length: u64,
        expected_version: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if expected_length != self.length || expected_version != self.version {
            return Err(failure("Relative resource revision changed"));
        }
        // read_at checks the same retained source and every containing capability before/after I/O.
        self.read_at(offset, length)
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if length == 0 || length > MAX_READ || offset.checked_add(u64::from(length)).is_none() {
            return Err(failure("Invalid relative resource read bounds"));
        }
        let cancel = Cancellation::default();
        self.validate_parent(&cancel)?;
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| failure("Relative resource lock failed"))?;
        handles.validate(&cancel)?;
        let wanted = self.length.saturating_sub(offset).min(u64::from(length)) as usize;
        handles
            .source
            .retry_read_from(offset.min(self.length), &cancel)
            .map_err(converted)?;
        let mut bytes = vec![0; wanted];
        let mut filled = 0;
        while filled < wanted {
            let count = handles
                .source
                .read_chunk(&mut bytes[filled..], &cancel)
                .map_err(converted)?;
            if count == 0 || count > wanted - filled {
                return Err(failure(
                    "Relative resource returned a short or invalid read",
                ));
            }
            filled += count;
        }
        handles.validate(&cancel)?;
        drop(handles);
        self.validate_parent(&cancel)?;
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "content_resources_tests.rs"]
mod tests;
