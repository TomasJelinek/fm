use super::*;
use crate::api::{Address, AddressEncoding};
use resource::{DISPLAY_SERVER, Resource};
use std::time::{Duration, Instant};
use windows_sys::Win32::NetworkManagement::WNet::*;

fn context() -> Context<'static> {
    Context {
        cancelled: &|| false,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
fn server() -> Resource {
    Resource {
        scope: RESOURCE_GLOBALNET,
        kind: RESOURCETYPE_DISK,
        display_type: DISPLAY_SERVER,
        usage: RESOURCEUSAGE_CONTAINER,
        remote: Some(wide(r"\\server")),
        provider: Some(wide("Provider A")),
        ..Default::default()
    }
}

#[test]
fn identity_preserves_every_native_field_and_unpaired_utf16_without_display_roundtrip() {
    let mut value = server();
    value.remote = Some(vec![92, 92, 0xd800, 0x6587]);
    value.local = Some(vec![]);
    value.comment = Some(wide("same display"));
    let key = value.encode().unwrap();
    assert_eq!(Resource::decode(&key).unwrap(), value);
    let record = value.record().unwrap().unwrap();
    assert_eq!(
        Resource::decode(&record.target.key).unwrap().remote,
        value.remote
    );
    assert!(record.display.contains('\u{fffd}'));
    let mut changed = value.clone();
    changed.provider = Some(wide("Provider B"));
    assert_ne!(key, changed.encode().unwrap());
    changed = value.clone();
    changed.scope = RESOURCE_CONTEXT;
    assert_ne!(key, changed.encode().unwrap());
    changed = value.clone();
    changed.local = None;
    assert_ne!(key, changed.encode().unwrap());
}

#[test]
fn malformed_or_excessive_keys_are_refused_without_truncating_identity() {
    let key = server().encode().unwrap();
    for length in 0..key.len() {
        assert!(Resource::decode(&key[..length]).is_err());
    }
    let mut trailing = key.clone();
    trailing.push(0);
    assert!(Resource::decode(&trailing).is_err());
    let mut value = server();
    value.remote = Some(vec![120; resource::MAX_KEY_BYTES]);
    assert_eq!(value.encode().unwrap_err().code, ErrorCode::ResourceLimit);
    value.remote = Some(vec![92, 92, 120, 0, 121]);
    assert!(value.encode().is_err());
}

#[test]
fn manual_servers_are_ready_inside_provider_and_shares_handoff_exact_native_unc_only_on_activation()
{
    for text in [
        "server",
        r"\\server",
        r"\\server\文档",
        r"\\server\文档\nested",
    ] {
        let record = Windows::manual(text).unwrap();
        Windows::validate(&record.target).unwrap();
        let expected = if text == "server" { r"\\server" } else { text };
        let resource = Resource::decode(&record.target.key).unwrap();
        assert_eq!(resource.remote, Some(wide(expected)));
        let result = Windows::activate(&record.target, &context()).unwrap();
        if expected == r"\\server" {
            assert_eq!(record.target.kind, NodeKind::Server);
            assert_eq!(
                result,
                ActivationTarget::WithinProvider(Address {
                    encoding: AddressEncoding::Opaque,
                    bytes: record.target.key
                })
            );
        } else {
            assert_eq!(record.target.kind, NodeKind::Share);
            assert_eq!(
                result,
                ActivationTarget::Native(Address {
                    encoding: AddressEncoding::WindowsUtf16Le,
                    bytes: wide(expected)
                        .into_iter()
                        .flat_map(u16::to_le_bytes)
                        .collect()
                })
            );
            assert!(
                Windows::begin(&record.target, &context()).is_err(),
                "listing a share must never open its filesystem"
            );
        }
    }
}

#[test]
fn manual_parser_rejects_urls_credentials_devices_dot_segments_and_ambiguous_separators() {
    for text in [
        "",
        "  ",
        "smb://server/share",
        "user@server",
        r"C:\share",
        r"\\.\pipe",
        r"\\?\UNC\host\share",
        r"\host",
        r"\\host\",
        r"\\host\\share",
        r"\\host\..\share",
        r"\\host\share\..",
        "\\\\host\\share\0hidden",
        r"//host/share",
    ] {
        assert!(Windows::manual(text).is_err(), "must reject {text:?}");
    }
}

#[test]
fn native_unc_preserves_unpaired_utf16_and_paths_beyond_max_path() {
    let mut units = wide(r"\\server\share\");
    units.push(0xd800);
    units.extend(vec![120; 4000]);
    let bytes = path::native(&units).unwrap();
    assert_eq!(
        bytes,
        units
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>()
    );
    let record = Windows::manual(&format!("\\\\server\\{}", "文".repeat(4000))).unwrap();
    assert_eq!(record.target.kind, NodeKind::Share);
}

#[test]
fn root_kind_and_resource_kind_cannot_be_forged_and_activation_checks_cancel_deadline() {
    let root = Windows::root();
    Windows::validate(&root.target).unwrap();
    let mut forged = root.target;
    forged.kind = NodeKind::Share;
    assert!(Windows::validate(&forged).is_err());
    let record = Windows::manual(r"\\server\share").unwrap();
    let cancelled = Context {
        cancelled: &|| true,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    assert_eq!(
        Windows::activate(&record.target, &cancelled)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    let expired = Context {
        cancelled: &|| false,
        deadline: Instant::now() - Duration::from_secs(1),
    };
    assert_eq!(
        Windows::activate(&record.target, &expired)
            .unwrap_err()
            .code,
        ErrorCode::TimedOut
    );
}

#[test]
fn unsupported_protocols_remain_explicit_and_share_names_may_contain_at_signs() {
    assert_eq!(
        Windows::manual("ftp://host/share").unwrap_err().code,
        ErrorCode::Unsupported
    );
    let share = Windows::manual(r"\\host\name@example.com").unwrap();
    assert!(matches!(
        Windows::activate(&share.target, &context()).unwrap(),
        ActivationTarget::Native(_)
    ));
    let mut resource = server();
    resource.display_type = resource::DISPLAY_SHARE;
    resource.usage = RESOURCEUSAGE_CONNECTABLE;
    resource.remote = Some(wide("non-UNC-provider-target"));
    let target = resource.record().unwrap().unwrap().target;
    assert_eq!(
        Windows::activate(&target, &context()).unwrap_err().code,
        ErrorCode::Unsupported
    );
}
