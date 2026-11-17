use midnight_filesystem_plugin_api::abi::*;

#[test]
fn public_layout_is_explicit_for_supported_64_bit_targets() {
    assert_eq!(size_of::<usize>(), 8);
    assert_eq!(size_of::<Slice>(), 16);
    assert_eq!(size_of::<Buffer>(), 24);
    assert_eq!(size_of::<Header>(), 16);
    assert_eq!(size_of::<Cancellation>(), 16);
    assert_eq!(size_of::<Call>(), 40);
    assert_eq!(size_of::<Config>(), 40);
    assert_eq!(size_of::<Api>(), 88);
    assert_eq!(std::mem::offset_of!(Api, capabilities), 16);
    assert_eq!(std::mem::offset_of!(Api, create), 56);
}

#[test]
fn zero_initialized_tables_are_representable_but_not_accepted_as_live_plugins() {
    let api = Api::default();
    assert!(api.create.is_none());
    assert!(api.call.is_none());
    assert!(api.release_buffer.is_none());
    assert!(api.destroy.is_none());
    assert!(api.validate(size_of::<Api>() as u32).is_err());
    assert!(Cancellation::default().cancelled.is_none());
}

#[test]
fn caller_owned_capacity_reserved_bytes_and_foreign_versions_are_checked() {
    let header = Header::v1(size_of::<Api>() as u32);
    assert_eq!(
        header.validate(size_of::<Api>() as u32, size_of::<Api>() as u32),
        Ok(())
    );
    assert_eq!(
        Header {
            abi_minor: 0,
            minimum_host_minor: 0,
            ..header
        }
        .validate(88, 88),
        Ok(())
    );
    assert!(header.validate(16, size_of::<Api>() as u32).is_err());
    assert!(
        Header {
            reserved: 1,
            ..header
        }
        .validate(88, 88)
        .is_err()
    );
    assert!(
        Header {
            abi_major: 2,
            ..header
        }
        .validate(88, 88)
        .is_err()
    );
    assert!(
        Header {
            minimum_host_minor: ABI_MINOR + 1,
            ..header
        }
        .validate(88, 88)
        .is_err()
    );
}

#[test]
fn borrowed_memory_metadata_and_config_limits_fail_closed() {
    assert!(
        Slice {
            data: std::ptr::null(),
            len: 1
        }
        .validate(8)
        .is_err()
    );
    assert!(Slice::default().validate(0).is_ok());
    assert!(
        Buffer {
            id: 0,
            bytes: Slice {
                data: b"x".as_ptr(),
                len: 1
            }
        }
        .validate(8)
        .is_err()
    );
    let config = Config::default();
    assert!(config.validate().is_ok());
    assert!(
        Config {
            request_bytes: u64::MAX,
            ..config
        }
        .validate()
        .is_err()
    );
    assert!(
        Config {
            max_calls: 0,
            ..config
        }
        .validate()
        .is_err()
    );
    assert!(
        Config {
            reserved: 1,
            ..config
        }
        .validate()
        .is_err()
    );
}

extern "C" fn create(_: *const Config, _: *mut u64) -> Status {
    INVALID
}
extern "C" fn call(_: u64, _: *const Call, _: *mut Buffer) -> Status {
    INVALID
}
extern "C" fn release(_: u64, _: u64) -> Status {
    INVALID
}
extern "C" fn destroy(_: u64) -> Status {
    INVALID
}
/// A complete hand-written descriptor, as a C plugin or another host would build it.
fn descriptor(capabilities: u64, minor: u16) -> Api {
    Api {
        header: Header {
            abi_minor: minor,
            minimum_host_minor: minor,
            ..Header::v1(size_of::<Api>() as u32)
        },
        capabilities,
        id: Slice {
            data: b"test.fs".as_ptr(),
            len: 7,
        },
        version: Slice {
            data: b"1".as_ptr(),
            len: 1,
        },
        create: Some(create),
        call: Some(call),
        release_buffer: Some(release),
        destroy: Some(destroy),
    }
}

#[test]
fn extensions_require_their_defining_minor_and_unknown_capabilities_are_incompatible() {
    let size = size_of::<Api>() as u32;
    let legacy = CAP_BROWSE
        | CAP_READ
        | CAP_STAT
        | CAP_VOLUMES
        | CAP_STORAGE
        | CAP_NATIVE_ADDRESS
        | CAP_VOLUME_DETAILS;
    assert_eq!(descriptor(legacy, 0).validate(size), Ok(()));
    for capability in [CAP_ACTIVATE, CAP_REFRESH, CAP_RESOLVE_TEXT] {
        assert_eq!(
            descriptor(CAP_BROWSE | capability, 2).validate(size),
            Ok(())
        );
        let low_host = descriptor(CAP_BROWSE | capability, 1);
        assert_eq!(low_host.validate(size), Err(INCOMPATIBLE));
        let mut old_layout = descriptor(CAP_BROWSE | capability, 2);
        old_layout.header.abi_minor = 1;
        assert_eq!(old_layout.validate(size), Err(INCOMPATIBLE));
    }
    assert_eq!(descriptor(CAP_PREPARED_RENAME, 3).validate(size), Ok(()));
    assert_eq!(
        descriptor(CAP_PREPARED_RENAME | CAP_ACTIVATE, 2).validate(size),
        Err(INCOMPATIBLE)
    );
    for unknown in [1 << 11, 1 << 20, 1 << 63] {
        assert_eq!(
            descriptor(legacy | unknown, ABI_MINOR).validate(size),
            Err(INCOMPATIBLE),
            "{unknown:#x}"
        );
    }
}

#[test]
fn prepared_rename_is_additive_and_requires_an_honest_minor_three_descriptor() {
    assert_eq!(ABI_MAJOR, 1);
    assert_eq!(ABI_MINOR, 3);
    assert_eq!(CAP_PREPARED_RENAME, 1024);
    for minor in 0..=2 {
        assert_eq!(
            Header {
                abi_minor: minor,
                minimum_host_minor: minor,
                ..Header::v1(88)
            }
            .validate(88, 88),
            Ok(())
        );
    }
    let old = Header {
        abi_minor: 2,
        minimum_host_minor: 2,
        ..Header::v1(88)
    };
    assert_eq!(old.validate(88, 88), Ok(()));
    let header = include_str!("../../include/midnight_filesystem.h");
    for declaration in [
        "#define ME_FS_ABI_MINOR 3",
        "ME_FS_CAP_PREPARED_RENAME UINT64_C(1024)",
        "ME_FS_PREPARE_RENAME=20",
        "ME_FS_GET_RENAME_STATE=22",
    ] {
        assert!(header.contains(declaration), "{declaration}");
    }
}
