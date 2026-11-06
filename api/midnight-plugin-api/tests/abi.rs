use midnight_plugin_api::abi::*;

fn header() -> Header {
    Header {
        struct_size: size_of::<PluginApi>() as u32,
        abi_major: ABI_MAJOR,
        abi_minor: ABI_MINOR,
        kind: KIND_VIEWER,
        minimum_host_minor: 0,
        reserved: 0,
    }
}

#[test]
fn short_descriptors_and_unavailable_required_host_versions_are_rejected() {
    let required = size_of::<PluginApi>() as u32;
    for invalid in [
        Header {
            struct_size: 8,
            ..header()
        },
        Header {
            abi_major: 2,
            ..header()
        },
        Header {
            minimum_host_minor: 1,
            ..header()
        },
        Header {
            kind: KIND_FILESYSTEM,
            ..header()
        },
        Header {
            reserved: 1,
            ..header()
        },
    ] {
        assert!(invalid.validate(required, required, KIND_VIEWER).is_err());
    }
    assert!(
        header()
            .validate(required - 1, required, KIND_VIEWER)
            .is_err()
    );
}

#[test]
fn optional_future_descriptor_tails_do_not_require_reading_them() {
    let required = size_of::<PluginApi>() as u32;
    let future = Header {
        struct_size: required + 64,
        abi_minor: 3,
        ..header()
    };
    assert!(future.validate(required, required, KIND_VIEWER).is_ok());
}
