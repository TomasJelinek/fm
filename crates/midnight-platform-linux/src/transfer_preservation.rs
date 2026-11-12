//! Copy existing-output access controls onto an unnamed inode before its first byte.
use super::*;
use rustix::fs::{
    IFlags, XattrFlags, fchmod, fchown, fgetxattr, fgetxattr_query, flistxattr, flistxattr_query,
    fremovexattr, fsetxattr, ioctl_getflags,
};
use std::collections::BTreeMap;

// Metadata resource limits only; paths, filenames and output contents are not truncated.
const MAX_ATTRIBUTE_BYTES: usize = 64 * 1024;
const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct Snapshot {
    owner: u32,
    group: u32,
    mode: u32,
    flags: IFlags,
    attributes: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl Snapshot {
    pub(super) fn capture(
        directory: &Directory,
        name: &OsStr,
        expected: &dyn TransferTarget,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        directory.revalidate(cancel)?;
        validate_name(name, &directory.path)?;
        let path = directory.path.join(name);
        if expected.entry().item.path != path || expected.entry().item.kind != EntryKind::File {
            return Err(invalid(
                &path,
                "preservation requires the consented regular file",
            ));
        }
        expected.revalidate(cancel)?;
        let file = File::from(
            openat2(
                &*directory.file,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                RESOLVE,
            )
            .map_err(|e| error(&path, e.into()))?,
        );
        let metadata = file.metadata().map_err(|e| error(&path, e))?;
        if identity(&file, &path)? != expected.entry().identity || !metadata.is_file() {
            return Err(invalid(
                &path,
                "existing output changed before metadata capture",
            ));
        }
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o7000 != 0 {
            return Err(unsupported(
                &path,
                "Existing output must be owned by this user and have no privileged mode bits; replacement refused.",
            ));
        }
        let attributes = attributes(&file, &path)?;
        if attributes
            .keys()
            .any(|name| name.starts_with(b"security.") && name != b"security.selinux")
        {
            return Err(unsupported(
                &path,
                "Existing output has security capabilities or integrity metadata that cannot survive changing its contents.",
            ));
        }
        let snapshot = Self {
            owner: metadata.uid(),
            group: metadata.gid(),
            mode: metadata.mode() & 0o777,
            flags: ioctl_getflags(&file).map_err(|e| error(&path, e.into()))?,
            attributes,
        };
        expected.revalidate(cancel)?;
        Ok(snapshot)
    }

    pub(super) fn initialize(&self, file: &File, path: &Path) -> Result<(), FsError> {
        let metadata = file.metadata().map_err(|e| error(path, e))?;
        if metadata.uid() != self.owner {
            return Err(unsupported(
                path,
                "New output owner differs from the original; replacement refused.",
            ));
        }
        if metadata.gid() != self.group {
            fchown(file, None, Some(rustix::process::Gid::from_raw(self.group)))
                .map_err(|e| error(path, e.into()))?;
        }
        fchmod(file, Mode::from_raw_mode(self.mode)).map_err(|e| error(path, e.into()))?;
        let inherited = attributes(file, path)?;
        for name in inherited
            .keys()
            .filter(|name| !self.attributes.contains_key(*name))
        {
            // Only an unnamed, exclusively owned stage is modified. A mandatory inherited
            // security attribute that cannot be removed causes a safe refusal.
            fremovexattr(file, OsStr::from_bytes(name)).map_err(|e| error(path, e.into()))?;
        }
        for (name, value) in &self.attributes {
            if inherited.get(name) != Some(value) {
                fsetxattr(file, OsStr::from_bytes(name), value, XattrFlags::empty())
                    .map_err(|e| error(path, e.into()))?;
            }
        }
        self.verify(file, path)
    }

    pub(super) fn verify(&self, file: &File, path: &Path) -> Result<(), FsError> {
        let metadata = file.metadata().map_err(|e| error(path, e))?;
        if metadata.uid() != self.owner
            || metadata.gid() != self.group
            || metadata.mode() & 0o7777 != self.mode
            || ioctl_getflags(file).map_err(|e| error(path, e.into()))? != self.flags
            || attributes(file, path)? != self.attributes
        {
            return Err(unsupported(
                path,
                "Existing output permissions, extended attributes or filesystem flags could not be preserved exactly.",
            ));
        }
        Ok(())
    }
}

fn attributes(file: &File, path: &Path) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, FsError> {
    let count = flistxattr_query(file).map_err(|e| error(path, e.into()))?;
    if count > MAX_ATTRIBUTE_BYTES {
        return Err(unsupported(
            path,
            "Existing output attribute names exceed the metadata budget.",
        ));
    }
    let mut names = vec![0u8; count];
    let read = flistxattr(file, names.as_mut_slice()).map_err(|e| error(path, e.into()))?;
    if count != read || count > 0 && names.last() != Some(&0) {
        return Err(invalid(
            path,
            "existing output attributes changed while reading",
        ));
    }
    let mut total = count;
    let mut result = BTreeMap::new();
    for name in names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let native = OsStr::from_bytes(name);
        let count = fgetxattr_query(file, native).map_err(|e| error(path, e.into()))?;
        total = total
            .checked_add(count)
            .ok_or_else(|| unsupported(path, "attribute size overflow"))?;
        if count > MAX_ATTRIBUTE_BYTES || total > MAX_METADATA_BYTES {
            return Err(unsupported(
                path,
                "Existing output attributes exceed the metadata budget.",
            ));
        }
        let mut value = vec![0u8; count];
        let read =
            fgetxattr(file, native, value.as_mut_slice()).map_err(|e| error(path, e.into()))?;
        if count != read || result.insert(name.to_vec(), value).is_some() {
            return Err(invalid(
                path,
                "existing output attributes changed while reading",
            ));
        }
    }
    Ok(result)
}
