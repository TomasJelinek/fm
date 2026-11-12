use crate::{policy, system};
use midnight_filesystem_plugin_api::*;
use midnight_filesystem_plugin_sdk::CallContext;
use std::{collections::HashSet, path::Path};

fn field(value: &[libc::c_char]) -> Result<Vec<u8>, Failure> {
    policy::native_field(&value.iter().map(|byte| *byte as u8).collect::<Vec<_>>())
}
fn text(value: &[libc::c_char]) -> Option<NativeText> {
    field(value)
        .ok()
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| NativeText {
            encoding: NameEncoding::UnixBytes,
            bytes,
        })
}
fn root(value: &libc::statfs) -> Result<Address, Failure> {
    let bytes = field(&value.f_mntonname)?;
    if !bytes.starts_with(b"/") {
        return Err(Failure::new(
            ErrorCode::Invalid,
            "Native mount root is not absolute.",
        ));
    }
    Ok(Address {
        encoding: AddressEncoding::UnixBytes,
        bytes,
    })
}

pub(crate) fn list(context: &CallContext<'_>) -> Result<Vec<Volume>, Failure> {
    system::cancel(context)?;
    // SAFETY: null/zero explicitly requests the count; NOWAIT avoids probing every mounted server.
    let initial = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if initial < 0 {
        return Err(system::os_error());
    }
    let snapshot = policy::mount_snapshot(initial as usize, |capacity| {
        system::cancel(context)?;
        let bytes = capacity
            .checked_mul(size_of::<libc::statfs>())
            .and_then(|bytes| i32::try_from(bytes).ok())
            .ok_or_else(|| {
                Failure::new(
                    ErrorCode::ResourceLimit,
                    "Mount buffer exceeds native capacity.",
                )
            })?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity).map_err(|_| {
            Failure::new(
                ErrorCode::ResourceLimit,
                "Mount allocation budget exceeded.",
            )
        })?;
        // SAFETY: statfs contains scalar/array output fields and zero is a valid initial representation.
        values.resize_with(capacity, || unsafe { std::mem::zeroed::<libc::statfs>() });
        // SAFETY: initialized array exactly covers bytes; the kernel retains no pointer after return.
        let count = unsafe { libc::getfsstat(values.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
        if count < 0 {
            return Err(system::os_error());
        }
        system::cancel(context)?;
        if count as usize > capacity {
            return Err(Failure::new(
                ErrorCode::ResourceLimit,
                "Mount count exceeded its supplied buffer.",
            ));
        }
        values.truncate(count as usize);
        Ok(values)
    })?;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for mount in snapshot {
        system::cancel(context)?;
        let root = root(&mount)?;
        if !seen.insert(root.bytes.clone()) {
            continue;
        }
        let id = format!(
            "mount:{}",
            root.bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        result.push(Volume {
            id,
            display: String::from_utf8_lossy(&root.bytes).into_owned(),
            root,
        });
    }
    Ok(result)
}

pub(crate) fn storage(path: &Path, context: &CallContext<'_>) -> Result<StorageKind, Failure> {
    let value = system::path_filesystem(path, context)?;
    Ok(policy::storage(&field(&value.f_fstypename)?))
}

pub(crate) fn information(
    path: &Path,
    context: &CallContext<'_>,
) -> Result<VolumeInformation, Failure> {
    let value = system::path_filesystem(path, context)?;
    let mut unavailable = vec!["Native volume label is not available from statfs.".into()];
    let root = match root(&value) {
        Ok(root) => Some(root),
        Err(error) => {
            unavailable.push(error.message);
            None
        }
    };
    let filesystem = text(&value.f_fstypename);
    let storage = filesystem
        .as_ref()
        .map_or(StorageKind::Unknown, |value| policy::storage(&value.bytes));
    let capacity = policy::capacity(
        u64::from(value.f_bsize),
        value.f_blocks,
        value.f_bfree,
        value.f_bavail,
    );
    if capacity.total.is_none() || capacity.free.is_none() || capacity.available.is_none() {
        unavailable.push("Some filesystem capacity values are unavailable or inconsistent.".into());
    }
    let mut properties = Vec::new();
    for (flag, property) in [
        (libc::MNT_NOEXEC, VolumeProperty::NoExecute),
        (libc::MNT_NOSUID, VolumeProperty::NoSetId),
        (libc::MNT_NODEV, VolumeProperty::NoDeviceFiles),
        (libc::MNT_QUOTA, VolumeProperty::Quotas),
    ] {
        if value.f_flags & flag as u32 != 0 {
            properties.push(property);
        }
    }
    Ok(VolumeInformation {
        root,
        label: None,
        filesystem,
        source: text(&value.f_mntfromname),
        capacity,
        storage,
        read_only: Some(value.f_flags & libc::MNT_RDONLY as u32 != 0),
        properties,
        unavailable,
    })
}
