//! Constant-space, order-independent immediate-directory observation.
use midnight_core::{Cancellation, CompressionStatus, Entry, EntryKind, FileSystem};
use sha2::{Digest, Sha256};
use std::{
    path::{Component, Path},
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Signature {
    sum: [u8; 32],
    count: u64,
}

impl Signature {
    pub(crate) fn add(&mut self, entry: &Entry) -> Result<(), ()> {
        let mut row = Sha256::new();
        row.update(b"midnight.watch.row.v1");
        let name = entry.name.as_encoded_bytes();
        row.update((name.len() as u64).to_le_bytes());
        row.update(name);
        row.update([match entry.kind {
            EntryKind::Directory => 0,
            EntryKind::DirectoryLink => 1,
            EntryKind::File => 2,
            EntryKind::Symlink => 3,
            EntryKind::Other => 4,
        }]);
        let size = (entry.kind == EntryKind::File)
            .then_some(entry.metadata.size)
            .flatten();
        field(&mut row, 1, size);
        row.update([2]);
        match entry.metadata.modified {
            None => row.update([0]),
            Some(time) => {
                let (sign, duration) = match time.duration_since(UNIX_EPOCH) {
                    Ok(duration) => (1, duration),
                    Err(error) => (2, error.duration()),
                };
                row.update([sign]);
                row.update(duration.as_secs().to_le_bytes());
                row.update(duration.subsec_nanos().to_le_bytes());
            }
        }
        row.update([
            3,
            match entry.metadata.compression {
                CompressionStatus::Unknown => 0,
                CompressionStatus::Uncompressed => 1,
                CompressionStatus::Compressed => 2,
            },
        ]);
        field(&mut row, 4, entry.metadata.native_attributes.map(u64::from));
        field(&mut row, 5, entry.metadata.unix_mode.map(u64::from));
        let digest = row.finalize();
        let mut carry = 0u16;
        for (sum, value) in self.sum.iter_mut().zip(digest) {
            let total = u16::from(*sum) + u16::from(value) + carry;
            *sum = total as u8;
            carry = total >> 8;
        }
        self.count = self.count.checked_add(1).ok_or(())?;
        Ok(())
    }

    pub(crate) fn digest(self, folders_only: bool) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"midnight.watch.multiset.v1");
        hash.update([u8::from(folders_only)]);
        hash.update(self.count.to_le_bytes());
        hash.update(self.sum);
        hash.finalize().into()
    }
}

fn field(hash: &mut Sha256, tag: u8, value: Option<u64>) {
    hash.update([tag, u8::from(value.is_some())]);
    if let Some(value) = value {
        hash.update(value.to_le_bytes());
    }
}

pub(crate) struct Observation {
    pub(crate) full: [u8; 32],
    pub(crate) display: [u8; 32],
}

pub(crate) fn probe(
    filesystem: &dyn FileSystem,
    path: &Path,
    folders_only: bool,
    cancel: &Cancellation,
) -> Result<Observation, ()> {
    let mut full = Signature::default();
    let mut display = Signature::default();
    let mut failed = false;
    filesystem
        .read_directory(path, cancel, &mut |mut entry| {
            if cancel.is_cancelled() {
                return false;
            }
            let valid = entry.path.strip_prefix(path).ok().is_some_and(|relative| {
                let mut parts = relative.components();
                matches!(parts.next(), Some(Component::Normal(name)) if name == entry.name)
                    && parts.next().is_none()
            });
            if !valid || entry.virtual_node.is_some() || entry.network_node.is_some() {
                failed = true;
                return false;
            }
            match filesystem.entry_metadata(&entry.path) {
                Ok(metadata) => entry.metadata = metadata,
                Err(_) => {
                    failed = true;
                    return false;
                }
            }
            if full.add(&entry).is_err()
                || ((!folders_only
                    || matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink))
                    && display.add(&entry).is_err())
            {
                failed = true;
                return false;
            }
            !cancel.is_cancelled()
        })
        .map_err(|_| ())?;
    if failed || cancel.is_cancelled() {
        return Err(());
    }
    Ok(Observation {
        full: full.digest(false),
        display: display.digest(folders_only),
    })
}

#[cfg(test)]
#[path = "signature_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "signature_linux_tests.rs"]
mod linux_tests;
#[cfg(all(test, windows))]
#[path = "signature_windows_tests.rs"]
mod windows_tests;
