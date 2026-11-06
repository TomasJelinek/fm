use midnight_plugin_api::abi::*;

fn bytes(text: &'static [u8]) -> ByteSlice {
    ByteSlice {
        data: text.as_ptr(),
        len: text.len() as u32,
        reserved: 0,
    }
}

fn header(size: usize, kind: u32) -> Header {
    Header {
        struct_size: size as u32,
        abi_major: 1,
        abi_minor: 0,
        kind,
        minimum_host_minor: 0,
        reserved: 0,
    }
}

extern "C" fn create(_: *const HostApi, _: ByteSlice, _: *mut u64) -> Status {
    OK
}
extern "C" fn submit(_: u64, _: *const Request) -> Status {
    OK
}
extern "C" fn pair(_: u64, _: u64) -> Status {
    OK
}
extern "C" fn single(_: u64) -> Status {
    OK
}
extern "C" fn drain(_: u64, _: u32, _: *mut Event, _: *mut u32) -> Status {
    OK
}
extern "C" fn schedule(_: u64, _: *const Work) -> Status {
    OK
}
extern "C" fn notify(_: u64) {}
extern "C" fn diagnostic(_: u64, _: u32, _: ByteSlice) {}

#[test]
fn required_plugin_callbacks_and_metadata_are_checked_before_calls() {
    let table = PluginApi {
        header: header(size_of::<PluginApi>(), KIND_VIEWER),
        id: bytes(b"test.viewer"),
        version: bytes(b"0.1.0"),
        create: Some(create),
        submit: Some(submit),
        cancel: Some(pair),
        drain: Some(drain),
        release_buffer: Some(pair),
        begin_shutdown: Some(single),
        destroy: Some(single),
    };
    let size = size_of::<PluginApi>() as u32;
    assert!(table.validate(size, KIND_VIEWER).is_ok());
    for invalid in [
        PluginApi {
            create: None,
            ..table
        },
        PluginApi {
            submit: None,
            ..table
        },
        PluginApi {
            cancel: None,
            ..table
        },
        PluginApi {
            drain: None,
            ..table
        },
        PluginApi {
            release_buffer: None,
            ..table
        },
        PluginApi {
            begin_shutdown: None,
            ..table
        },
        PluginApi {
            destroy: None,
            ..table
        },
        PluginApi {
            id: bytes(b""),
            ..table
        },
    ] {
        assert!(invalid.validate(size, KIND_VIEWER).is_err());
    }
}

#[test]
fn required_host_services_cannot_be_missing() {
    let table = HostApi {
        header: header(size_of::<HostApi>(), KIND_HOST),
        context: 42,
        schedule_work: Some(schedule),
        cancelled: Some(pair),
        submit_service: Some(submit),
        notify_ready: Some(notify),
        retain_buffer: Some(pair),
        release_buffer: Some(pair),
        diagnostic: Some(diagnostic),
    };
    let size = size_of::<HostApi>() as u32;
    assert!(table.validate(size).is_ok());
    for invalid in [
        HostApi {
            schedule_work: None,
            ..table
        },
        HostApi {
            cancelled: None,
            ..table
        },
        HostApi {
            submit_service: None,
            ..table
        },
        HostApi {
            notify_ready: None,
            ..table
        },
        HostApi {
            retain_buffer: None,
            ..table
        },
        HostApi {
            release_buffer: None,
            ..table
        },
        HostApi {
            diagnostic: None,
            ..table
        },
    ] {
        assert!(invalid.validate(size).is_err());
    }
}

#[test]
fn malformed_buffer_views_are_rejected_without_dereferencing() {
    let empty = ByteSlice {
        data: std::ptr::null(),
        len: 0,
        reserved: 0,
    };
    assert!(empty.validate(100).is_ok());
    for invalid in [
        ByteSlice { len: 1, ..empty },
        ByteSlice {
            reserved: 1,
            ..empty
        },
        ByteSlice {
            len: u32::MAX,
            ..bytes(b"x")
        },
    ] {
        assert!(invalid.validate(100).is_err());
    }
    assert!(
        Buffer {
            bytes: bytes(b"content"),
            id: 0
        }
        .validate(100)
        .is_err()
    );
    assert!(
        Buffer {
            bytes: empty,
            id: 0
        }
        .validate(100)
        .is_ok()
    );
}
