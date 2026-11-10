use midnight_filesystem_plugin_api::{ErrorCode, Failure, StorageKind, VolumeCapacity};

pub(crate) fn native_field(bytes: &[u8]) -> Result<Vec<u8>, Failure> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| Failure::new(ErrorCode::Invalid, "Unterminated native volume text."))?;
    // A fully occupied native field may have been truncated by the kernel. Do
    // not manufacture a navigable identity from that ambiguous display text.
    if end + 1 == bytes.len() {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "Native volume text may be truncated.",
        ));
    }
    Ok(bytes[..end].to_vec())
}

pub(crate) fn capacity(unit: u64, blocks: u64, free: u64, available: u64) -> VolumeCapacity {
    let bytes = |count: u64| {
        if unit == 0 || count == u64::MAX {
            None
        } else {
            count.checked_mul(unit)
        }
    };
    let total = bytes(blocks);
    let bounded = |count| bytes(count).filter(|value| total.is_none_or(|total| *value <= total));
    VolumeCapacity {
        total,
        free: bounded(free),
        available: bounded(available),
    }
}

pub(crate) fn storage(filesystem: &[u8]) -> StorageKind {
    match filesystem {
        b"smbfs" | b"nfs" | b"afpfs" | b"webdav" => StorageKind::Network,
        _ => StorageKind::Unknown,
    }
}

pub(crate) fn mount_snapshot<T>(
    initial: usize,
    mut read: impl FnMut(usize) -> Result<Vec<T>, Failure>,
) -> Result<Vec<T>, Failure> {
    const MAX_MOUNTS: usize = 4096;
    let limited = || Failure::new(ErrorCode::ResourceLimit, "Mount snapshot budget exceeded.");
    if initial > MAX_MOUNTS {
        return Err(limited());
    }
    let mut capacity = initial.saturating_add(16).min(MAX_MOUNTS);
    for _ in 0..4 {
        let entries = read(capacity)?;
        if entries.len() < capacity {
            return Ok(entries);
        }
        if entries.len() > capacity || capacity == MAX_MOUNTS {
            return Err(limited());
        }
        capacity = capacity.saturating_mul(2).min(MAX_MOUNTS);
    }
    Err(limited())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_fields_preserve_raw_bytes_and_reject_possible_truncation() {
        assert_eq!(
            native_field(b"/Volumes/raw\xff\0\0").unwrap(),
            b"/Volumes/raw\xff"
        );
        assert!(native_field(b"unterminated").is_err());
        assert!(native_field(b"full\0").is_err());
        assert_eq!(native_field(b"\0\0").unwrap(), b"");
    }

    #[test]
    fn capacity_retains_independent_values_without_wraparound() {
        assert_eq!(
            capacity(4096, 10, 4, 3),
            VolumeCapacity {
                total: Some(40960),
                free: Some(16384),
                available: Some(12288)
            }
        );
        assert_eq!(
            capacity(2, u64::MAX, 4, 3),
            VolumeCapacity {
                total: None,
                free: Some(8),
                available: Some(6)
            }
        );
        assert_eq!(capacity(0, 10, 4, 3), VolumeCapacity::default());
        assert_eq!(
            capacity(1, 10, 11, 12),
            VolumeCapacity {
                total: Some(10),
                free: None,
                available: None
            }
        );
    }

    #[test]
    fn known_network_filesystems_never_become_local_ssd() {
        for name in [b"smbfs".as_slice(), b"nfs", b"afpfs", b"webdav"] {
            assert_eq!(storage(name), StorageKind::Network);
        }
        for name in [b"apfs".as_slice(), b"hfs", b"devfs", b"unknown"] {
            assert_eq!(storage(name), StorageKind::Unknown);
        }
    }

    #[test]
    fn saturated_mount_snapshot_is_retried_instead_of_published_as_complete() {
        let mut attempt = 0;
        let result = mount_snapshot(1, |capacity| {
            attempt += 1;
            Ok(if attempt == 1 {
                vec![0; capacity]
            } else {
                vec![1, 2, 3]
            })
        })
        .unwrap();
        assert_eq!(result, vec![1, 2, 3]);
        assert_eq!(attempt, 2);
    }

    #[test]
    fn unbounded_mount_churn_and_resource_exhaustion_fail_explicitly() {
        let failure = mount_snapshot(4096, |capacity| Ok(vec![0; capacity])).unwrap_err();
        assert_eq!(failure.code, ErrorCode::ResourceLimit);
        let failure = mount_snapshot::<()>(usize::MAX, |_| panic!("must reject before allocation"))
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::ResourceLimit);
        let failure =
            mount_snapshot::<()>(1, |_| Err(Failure::new(ErrorCode::Cancelled, "cancel")))
                .unwrap_err();
        assert_eq!(failure.code, ErrorCode::Cancelled);
    }
}
