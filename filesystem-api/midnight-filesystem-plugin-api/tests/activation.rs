use midnight_filesystem_plugin_api::{
    decode_request, decode_response, encode_request, encode_response,
};

fn payload() -> Vec<u8> {
    b"FSBR\x01\0\0\0".to_vec()
}

#[test]
fn explicit_activation_has_a_distinct_operation_and_preserves_captured_ids() {
    let mut bytes = payload();
    bytes.extend_from_slice(&91_u64.to_le_bytes());
    bytes.extend_from_slice(&27_u64.to_le_bytes());
    let request = decode_request(17, &bytes, 1024).expect("explicit activation must be supported");
    assert_eq!(request.operation() as u32, 17);
    assert_eq!(encode_request(&request, 1024).unwrap(), bytes);
    assert_ne!(
        decode_request(2, &bytes, 1024).unwrap().operation() as u32,
        17
    );
    bytes.extend_from_slice(&[0]);
    assert!(decode_request(17, &bytes, 1024).is_err());
}

fn target(kind: u8, encoding: u8, address: &[u8]) -> Vec<u8> {
    let mut bytes = payload();
    bytes.extend_from_slice(&14_u16.to_le_bytes());
    bytes.extend_from_slice(&[kind, encoding]);
    bytes.extend_from_slice(&(address.len() as u64).to_le_bytes());
    bytes.extend_from_slice(address);
    bytes
}

#[test]
fn native_handoff_preserves_raw_unicode_long_paths_and_provider_identity() {
    let unix = [
        b"/run/user/1000/gvfs/".as_slice(),
        &[0xff],
        "文档".as_bytes(),
    ]
    .concat();
    let windows: Vec<u8> = format!("\\\\server\\share\\{}", "文档\\".repeat(300))
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    for bytes in [
        target(2, 2, &unix),
        target(2, 1, &windows),
        target(1, 0, b"network\0scope\0server"),
    ] {
        let response = decode_response(&bytes, 32768).expect("typed handoff must be supported");
        assert_eq!(encode_response(&response, 32768).unwrap(), bytes);
        assert!(decode_response(&bytes, bytes.len() - 1).is_err());
    }
}

#[test]
fn native_handoff_rejects_virtual_relative_nul_and_malformed_addresses() {
    for bytes in [
        target(2, 0, b"network://server"),
        target(2, 2, b"smb://server/share"),
        target(2, 2, b"relative/share"),
        target(2, 2, b"/share\0other"),
        target(2, 1, &[b'C', 0, b':', 0, b'x', 0]),
        target(2, 1, &[b'\\', 0, b'\\']),
        target(3, 2, b"/unknown-target-kind"),
    ] {
        assert!(decode_response(&bytes, 32768).is_err());
    }
}

#[test]
fn activation_rejects_unissued_zero_handles() {
    for (mount, node) in [(0_u64, 1_u64), (1, 0)] {
        let mut bytes = payload();
        bytes.extend_from_slice(&mount.to_le_bytes());
        bytes.extend_from_slice(&node.to_le_bytes());
        assert!(decode_request(17, &bytes, 1024).is_err());
    }
}

#[test]
fn manual_input_and_refresh_do_not_overload_restore_or_activation() {
    let mut text = payload();
    let input = "server 文档".as_bytes();
    text.extend_from_slice(&(input.len() as u64).to_le_bytes());
    text.extend_from_slice(input);
    let manual = decode_request(19, &text, 1024).expect("manual provider input is distinct");
    assert_eq!(manual.operation() as u32, 19);
    assert_eq!(encode_request(&manual, 1024).unwrap(), text);
    let mut refresh = payload();
    refresh.extend_from_slice(&91_u64.to_le_bytes());
    refresh.extend_from_slice(&27_u64.to_le_bytes());
    let request = decode_request(18, &refresh, 1024).expect("refresh must invalidate TTL cache");
    assert_eq!(request.operation() as u32, 18);
    assert_eq!(encode_request(&request, 1024).unwrap(), refresh);
}

#[test]
fn network_failure_states_are_distinct_and_keep_existing_error_codes() {
    for code in 1_u16..=15 {
        let mut bytes = payload();
        bytes.extend_from_slice(&11_u16.to_le_bytes());
        bytes.extend_from_slice(&code.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        let response = decode_response(&bytes, 1024).unwrap();
        assert_eq!(encode_response(&response, 1024).unwrap(), bytes);
    }
}

#[test]
fn native_windows_handoff_rejects_devices_traversal_and_invalid_unc_components() {
    for path in [
        r"\\.\pipe\name",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1",
        r"\\?\UNC\?\share",
        r"\\server\share\..\other",
        r"\\server\share\.\other",
        r"\\server\share\\other",
        r"\\server\share\file:stream",
        r"\\server?\share",
        r"\\server\share*",
        r"C:\folder\..\other",
        r"\\?\C:\folder\..\other",
    ] {
        let address: Vec<_> = path.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert!(
            decode_response(&target(2, 1, &address), 32768).is_err(),
            "{path}"
        );
    }
}

#[test]
fn native_windows_handoff_preserves_verbatim_paths_and_unpaired_units() {
    for path in [
        r"C:\",
        r"C:/folder/file",
        r"\\server\share",
        r"\\?\UNC\server\share\",
        r"\\?\C:\folder\",
    ] {
        let mut address: Vec<_> = path.encode_utf16().flat_map(u16::to_le_bytes).collect();
        if path.ends_with("folder\\") {
            address.extend_from_slice(&0xd800_u16.to_le_bytes());
        }
        let bytes = target(2, 1, &address);
        let response = decode_response(&bytes, 32768).unwrap();
        assert_eq!(encode_response(&response, 32768).unwrap(), bytes);
    }
}
