#![cfg(windows)]
mod support;
use midnight_filesystem_plugin_api::{abi::*, *};
use support::{Client, Temp, address};

#[test]
fn optional_rich_volume_query_is_supported_by_native_provider() {
    let fixture = Temp::new();
    let client = Client::new();
    let payload = encode_request(
        &Request::SpaceInfo {
            address: address(&fixture.path),
        },
        DEFAULT_REQUEST_BYTES as usize,
    )
    .unwrap();
    let call = Call {
        struct_size: size_of::<Call>() as u32,
        operation: 16,
        payload: Slice {
            data: payload.as_ptr(),
            len: payload.len() as u64,
        },
        cancel: Cancellation::default(),
    };
    let mut buffer = Buffer::default();
    // SAFETY: request/output and client descriptor remain live for the synchronous call.
    let status = unsafe { client.api.call.unwrap()(client.instance, &call, &mut buffer) };
    assert_eq!(
        status, OK,
        "native plugin must support optional rich volume queries"
    );
    buffer.validate(DEFAULT_REPLY_BYTES).unwrap();
    // SAFETY: successful call owns this buffer until release through its original instance.
    let data = unsafe { std::slice::from_raw_parts(buffer.bytes.data, buffer.bytes.len as usize) };
    let response = decode_response(data, DEFAULT_REPLY_BYTES as usize);
    // SAFETY: successful buffer is released exactly once before the client is destroyed.
    assert_eq!(
        unsafe { client.api.release_buffer.unwrap()(client.instance, buffer.id) },
        OK
    );
    assert!(
        matches!(response, Ok(Response::VolumeInformation(_))),
        "{response:?}"
    );
}

#[test]
fn native_volume_details_preserve_capacity_label_and_failures() {
    let fixture = Temp::new();
    let client = Client::new();
    assert_ne!(client.api.capabilities & CAP_VOLUME_DETAILS, 0);
    let directory = fixture.path.join("Žluťoučký_文档_🦀");
    std::fs::create_dir(&directory).unwrap();
    let request = |path: &std::path::Path| Request::VolumeInformation {
        address: address(path),
    };
    let Response::VolumeInformation(info) = client.call(request(&directory)) else {
        panic!("Missing rich native information")
    };
    assert!(info.root.is_some());
    assert!(
        info.filesystem
            .as_ref()
            .is_some_and(|text| !text.bytes.is_empty())
    );
    assert!(info.capacity.total.is_some_and(|total| total > 0));
    assert!(info.capacity.free.unwrap() <= info.capacity.total.unwrap());
    assert!(info.capacity.available.is_some());
    assert!(info.read_only.is_some());
    assert_eq!(
        client.call(Request::VolumeLabel {
            address: address(&directory)
        }),
        Response::VolumeLabel(info.label)
    );
    assert!(matches!(
        client.call(request(&directory.join("missing"))),
        Response::Failed(Failure {
            code: ErrorCode::NotFound,
            ..
        })
    ));
    assert!(matches!(
        client.cancelled(request(&directory), 1),
        Response::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
}

#[test]
fn native_volume_details_accept_raw_and_long_directory_identities() {
    let fixture = Temp::new();
    let client = Client::new();
    let raw = fixture.path.join(raw_name());
    std::fs::create_dir(&raw).unwrap();
    let mut long = raw.clone();
    for _ in 0..5 {
        long = long.join("long_資料_".repeat(10));
    }
    std::fs::create_dir_all(&long).unwrap();
    for path in [&raw, &long] {
        assert!(matches!(
            client.call(Request::VolumeInformation {
                address: address(path)
            }),
            Response::VolumeInformation(_)
        ));
    }
}
fn raw_name() -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    std::ffi::OsString::from_wide(&[b'r' as u16, 0xd800])
}
