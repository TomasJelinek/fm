//! Every enumerated `ME_FS_*` value in the C header matches what the public codec emits.
use midnight_filesystem_plugin_api::{abi::*, *};
use std::collections::HashMap;

const LIMIT: usize = 1 << 20;

/// `NAME=value` enumerators of the C header (macros are checked in layout.rs).
fn header_values() -> HashMap<String, u64> {
    let header = include_str!("../../include/midnight_filesystem.h");
    let mut values = HashMap::new();
    for (start, _) in header.match_indices("ME_FS_") {
        let rest = &header[start..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let (name, tail) = rest.split_at(end);
        if let Some(number) = tail.strip_prefix('=') {
            let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
            let previous = values.insert(name.to_owned(), digits.parse().unwrap());
            assert!(previous.is_none(), "{name} is declared twice");
        }
    }
    values
}

fn text(value: &str) -> NativeText {
    NativeText {
        encoding: NameEncoding::Utf8,
        bytes: value.as_bytes().to_vec(),
    }
}
fn node(metadata: Metadata) -> Node {
    Node {
        id: 1,
        address: Address {
            encoding: AddressEncoding::UnixBytes,
            bytes: b"/x".to_vec(),
        },
        name_encoding: NameEncoding::Utf8,
        raw_name: b"x".to_vec(),
        display: "x".into(),
        kind: Kind::File,
        metadata,
    }
}
fn object() -> RenameObject {
    RenameObject {
        identity: vec![1],
        revision: vec![2],
        name: text("x"),
        kind: Kind::File,
        metadata: Metadata::default(),
    }
}
fn failure() -> Failure {
    Failure::new(ErrorCode::Io, "x")
}
fn native_root() -> Address {
    Address {
        encoding: AddressEncoding::UnixBytes,
        bytes: b"/".to_vec(),
    }
}
fn encode(response: &Response) -> Vec<u8> {
    encode_response(response, LIMIT).unwrap()
}

#[test]
fn c_header_values_match_the_public_codec() {
    let header = header_values();
    let mut checked = 0;
    let mut check = |name: String, actual: u64| {
        let declared = header
            .get(&name)
            .unwrap_or_else(|| panic!("{name} is missing"));
        assert_eq!(*declared, actual, "{name}");
        checked += 1;
    };

    // Response tags as encoded after the eight-byte FSBR prefix.
    let state = |attempt, outcome| {
        Response::RenameState(RenameState {
            prepared: 1,
            attempt,
            outcome,
        })
    };
    for (name, response) in [
        ("UNIT", Response::Unit),
        (
            "RESOLVED",
            Response::Resolved {
                mount: 1,
                node: node(Metadata::default()),
            },
        ),
        ("NODE", Response::Node(node(Metadata::default()))),
        (
            "PAGE",
            Response::Page {
                cursor: 1,
                page: 0,
                done: true,
                entries: vec![],
            },
        ),
        ("PARENT", Response::Parent(None)),
        (
            "OPENED",
            Response::Opened {
                reader: 1,
                length: 0,
                revision: vec![1],
            },
        ),
        (
            "CONTENT",
            Response::Content {
                bytes: vec![],
                eof: true,
            },
        ),
        ("VOLUMES", Response::Volumes(vec![])),
        ("STORAGE_KIND", Response::StorageKind(StorageKind::Unknown)),
        (
            "SPACE_INFO",
            Response::SpaceInfo {
                available: 0,
                total: 0,
            },
        ),
        ("FAILED", Response::Failed(failure())),
        ("VOLUME_LABEL", Response::VolumeLabel(None)),
        (
            "VOLUME_INFORMATION",
            Response::VolumeInformation(VolumeInformation::default()),
        ),
        (
            "ACTIVATED",
            Response::Activated(ActivationTarget::Native(native_root())),
        ),
        (
            "RENAME_PREPARED",
            Response::RenamePrepared(PreparedRename {
                handle: 1,
                source: object(),
                parent_identity: vec![3],
                recoverable_replace: false,
            }),
        ),
        ("RENAME_STATE", state(0, RenameOutcome::Ready)),
    ] {
        let bytes = encode(&response);
        let tag = u16::from_le_bytes([bytes[8], bytes[9]]);
        check(format!("ME_FS_REPLY_{name}"), tag.into());
    }

    // Byte values that follow fixed positions in their frames.
    for (name, target) in [
        (
            "WITHIN_PROVIDER",
            ActivationTarget::WithinProvider(native_root()),
        ),
        ("NATIVE", ActivationTarget::Native(native_root())),
    ] {
        let bytes = encode(&Response::Activated(target));
        check(format!("ME_FS_ACTIVATED_{name}"), bytes[10].into());
    }
    for (name, outcome) in [
        ("READY", RenameOutcome::Ready),
        ("EXECUTING", RenameOutcome::Executing),
        (
            "CONFLICT",
            RenameOutcome::Conflict(RenameConflict {
                token: 1,
                target: object(),
                can_replace: false,
            }),
        ),
        ("UNCHANGED", RenameOutcome::Unchanged(failure())),
        (
            "RENAMED",
            RenameOutcome::Renamed {
                name: text("y"),
                same_entry: false,
                recovery: vec![],
                durability_confirmed: true,
            },
        ),
        (
            "RECOVERY_REQUIRED",
            RenameOutcome::RecoveryRequired {
                source_location: RenameSourceLocation::Unknown,
                destination_name: None,
                recovery: vec![],
                error: failure(),
            },
        ),
    ] {
        let attempt = u64::from(name != "READY");
        // Prefix, tag, prepared handle and attempt precede the outcome byte.
        check(
            format!("ME_FS_OUTCOME_{name}"),
            encode(&state(attempt, outcome))[26].into(),
        );
    }
    for (name, policy, trailing) in [
        ("NO_REPLACE", RenamePolicy::NoReplace, 0),
        (
            "RECOVERABLE_CURRENT_ENTRY",
            RenamePolicy::RecoverableCurrentEntry { conflict: 1 },
            8,
        ),
    ] {
        let request = Request::Rename {
            prepared: 1,
            attempt: 1,
            name: text("y"),
            policy,
        };
        let bytes = encode_request(&request, LIMIT).unwrap();
        check(
            format!("ME_FS_POLICY_{name}"),
            bytes[bytes.len() - 1 - trailing].into(),
        );
    }
    // Metadata presence bits directly precede the trailing compression byte when
    // no optional field is present; a single present field keeps that position.
    let flags_at = encode(&Response::Node(node(Metadata::default()))).len() - 3;
    let time = Timestamp {
        seconds: 1,
        nanos: 0,
    };
    for (name, metadata) in [
        (
            "SIZE",
            Metadata {
                size: Some(1),
                ..Metadata::default()
            },
        ),
        (
            "MODIFIED",
            Metadata {
                modified: Some(time),
                ..Metadata::default()
            },
        ),
        (
            "NATIVE_ATTRIBUTES",
            Metadata {
                native_attributes: Some(1),
                ..Metadata::default()
            },
        ),
        (
            "UNIX_MODE",
            Metadata {
                unix_mode: Some(1),
                ..Metadata::default()
            },
        ),
    ] {
        let bytes = encode(&Response::Node(node(metadata)));
        let flags = u16::from_le_bytes([bytes[flags_at], bytes[flags_at + 1]]);
        check(format!("ME_FS_METADATA_{name}"), flags.into());
    }

    // Values with Rust discriminants.
    for (name, code) in [
        ("INVALID", ErrorCode::Invalid),
        ("UNSUPPORTED", ErrorCode::Unsupported),
        ("NOT_FOUND", ErrorCode::NotFound),
        ("PERMISSION_DENIED", ErrorCode::PermissionDenied),
        ("SOURCE_CHANGED", ErrorCode::SourceChanged),
        ("CANCELLED", ErrorCode::Cancelled),
        ("RESOURCE_LIMIT", ErrorCode::ResourceLimit),
        ("IO", ErrorCode::Io),
        ("STALE_REFERENCE", ErrorCode::StaleReference),
        ("BUSY", ErrorCode::Busy),
        ("UNAVAILABLE", ErrorCode::Unavailable),
        ("OFFLINE", ErrorCode::Offline),
        ("AUTHENTICATION_REQUIRED", ErrorCode::AuthenticationRequired),
        ("MOUNT_REQUIRED", ErrorCode::MountRequired),
        ("TIMED_OUT", ErrorCode::TimedOut),
    ] {
        check(format!("ME_FS_ERROR_{name}"), code as u64);
    }
    for (name, kind) in [
        ("DIRECTORY", Kind::Directory),
        ("FILE", Kind::File),
        ("DIRECTORY_LINK", Kind::DirectoryLink),
        ("SYMLINK", Kind::Symlink),
        ("OTHER", Kind::Other),
    ] {
        check(format!("ME_FS_NODE_{name}"), kind as u64);
    }
    for (name, encoding) in [
        ("OPAQUE", AddressEncoding::Opaque),
        ("WINDOWS_UTF16LE", AddressEncoding::WindowsUtf16Le),
        ("UNIX_BYTES", AddressEncoding::UnixBytes),
    ] {
        check(format!("ME_FS_ADDRESS_{name}"), encoding as u64);
    }
    for (name, encoding) in [
        ("OPAQUE", NameEncoding::Opaque),
        ("UTF8", NameEncoding::Utf8),
        ("WINDOWS_UTF16LE", NameEncoding::WindowsUtf16Le),
        ("UNIX_BYTES", NameEncoding::UnixBytes),
    ] {
        check(format!("ME_FS_NAME_{name}"), encoding as u64);
    }
    for (name, kind) in [
        ("UNKNOWN", StorageKind::Unknown),
        ("LOCAL_SSD", StorageKind::LocalSsd),
        ("ROTATIONAL", StorageKind::Rotational),
        ("NETWORK", StorageKind::Network),
    ] {
        check(format!("ME_FS_STORAGE_KIND_{name}"), kind as u64);
    }
    for (name, compression) in [
        ("UNKNOWN", Compression::Unknown),
        ("UNCOMPRESSED", Compression::Uncompressed),
        ("COMPRESSED", Compression::Compressed),
    ] {
        check(format!("ME_FS_COMPRESSION_{name}"), compression as u64);
    }
    for (name, property) in [
        ("CASE_PRESERVED_NAMES", VolumeProperty::CasePreservedNames),
        ("CASE_SENSITIVE_LOOKUP", VolumeProperty::CaseSensitiveLookup),
        ("UNICODE_NAMES", VolumeProperty::UnicodeNames),
        ("PERSISTENT_ACLS", VolumeProperty::PersistentAcls),
        ("FILE_COMPRESSION", VolumeProperty::FileCompression),
        ("SPARSE_FILES", VolumeProperty::SparseFiles),
        ("NAMED_STREAMS", VolumeProperty::NamedStreams),
        ("HARD_LINKS", VolumeProperty::HardLinks),
        ("REPARSE_POINTS", VolumeProperty::ReparsePoints),
        ("ENCRYPTION", VolumeProperty::Encryption),
        ("QUOTAS", VolumeProperty::Quotas),
        ("NO_EXECUTE", VolumeProperty::NoExecute),
        ("NO_SET_ID", VolumeProperty::NoSetId),
        ("NO_DEVICE_FILES", VolumeProperty::NoDeviceFiles),
    ] {
        check(format!("ME_FS_PROPERTY_{name}"), property as u64);
    }
    for (name, location) in [
        ("ORIGINAL_NAME", RenameSourceLocation::OriginalName),
        ("DESTINATION", RenameSourceLocation::Destination),
        ("RECOVERY", RenameSourceLocation::Recovery),
        ("UNKNOWN", RenameSourceLocation::Unknown),
    ] {
        check(format!("ME_FS_SOURCE_{name}"), location as u64);
    }
    for (name, role) in [
        ("SOURCE", RenameRecoveryRole::Source),
        ("DISPLACED_TARGET", RenameRecoveryRole::DisplacedTarget),
        ("EMPTY_STAGE", RenameRecoveryRole::EmptyStage),
    ] {
        check(format!("ME_FS_RECOVERY_{name}"), role as u64);
    }

    // Existing ABI statuses and operations share the header's enumerator syntax.
    for (name, status) in [
        ("OK", OK),
        ("INVALID", INVALID),
        ("INCOMPATIBLE", INCOMPATIBLE),
        ("BUSY", BUSY),
        ("UNSUPPORTED", UNSUPPORTED),
        ("RESOURCE_LIMIT", RESOURCE_LIMIT),
        ("UNKNOWN_HANDLE", UNKNOWN_HANDLE),
        ("SHUTTING_DOWN", SHUTTING_DOWN),
        ("PANICKED", PANICKED),
    ] {
        check(format!("ME_FS_{name}"), status.into());
    }
    for operation in [
        Operation::Resolve,
        Operation::Stat,
        Operation::ListBegin,
        Operation::ListNext,
        Operation::CloseList,
        Operation::Parent,
        Operation::OpenRead,
        Operation::ValidateRead,
        Operation::ReadAt,
        Operation::CloseRead,
        Operation::CloseMount,
        Operation::Volumes,
        Operation::StorageKind,
        Operation::SpaceInfo,
        Operation::VolumeLabel,
        Operation::VolumeInformation,
        Operation::Activate,
        Operation::Refresh,
        Operation::ResolveText,
        Operation::PrepareRename,
        Operation::Rename,
        Operation::GetRenameState,
        Operation::CloseRename,
    ] {
        // `ListBegin` is declared as `ME_FS_LIST_BEGIN`.
        let mut name = String::new();
        for (index, c) in format!("{operation:?}").char_indices() {
            if index > 0 && c.is_ascii_uppercase() {
                name.push('_');
            }
            name.push(c.to_ascii_uppercase());
        }
        check(format!("ME_FS_{name}"), u64::from(operation as u32));
    }

    // Nothing in the header is left unchecked.
    assert_eq!(checked, header.len());
}
