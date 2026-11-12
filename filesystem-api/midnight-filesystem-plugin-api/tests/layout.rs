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
