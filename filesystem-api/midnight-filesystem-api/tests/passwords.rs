use midnight_filesystem_api::*;

fn node(kind: Kind) -> Node {
    Node {
        id: 2,
        parent: 1,
        name: b"balcony.emc".to_vec(),
        display: "balcony.emc".into(),
        kind,
        size: Some(1368),
        readable: kind == Kind::File,
        modified: None,
        encryption: Encryption::None,
        contains_encrypted: false,
        unreadable: None,
    }
}

fn round_trip(protocol: Protocol, message: &Message) -> io::Result<Message> {
    let mut bytes = Vec::new();
    write_message_with_protocol(&mut bytes, protocol, message)?;
    read_message_with_protocol(&mut bytes.as_slice(), protocol)
}
use std::io;

#[test]
fn protocol_four_lists_encryption_and_reasons_and_unlocks() {
    let protocol = Protocol::try_from(4).unwrap();
    assert!(protocol.supports_staging() && protocol.cancels_members());
    assert!(protocol.supports_passwords());
    let mut encrypted = node(Kind::File);
    encrypted.encryption = Encryption::ZipCrypto;
    let mut locked = node(Kind::File);
    locked.encryption = Encryption::Aes256;
    locked.readable = false;
    locked.unreadable = Some(Unreadable::Encryption);
    let mut implode = node(Kind::File);
    implode.readable = false;
    implode.unreadable = Some(Unreadable::Compression("Implode".into()));
    let mut folder = node(Kind::Directory);
    folder.size = None;
    folder.contains_encrypted = true;
    let mut link = node(Kind::Link);
    link.unreadable = Some(Unreadable::Link);
    let messages = [
        Message::Entries {
            request: 3,
            entries: vec![encrypted, locked, implode, folder, link],
            more: false,
        },
        Message::Unlock {
            request: 4,
            password: Password::new("pässwörd".as_bytes().to_vec()),
        },
        Message::UnlockReply {
            request: 4,
            status: UnlockStatus::WrongPassword,
        },
        Message::Failed {
            request: 5,
            code: ErrorCode::PasswordRequired,
            message: "password required".into(),
        },
        Message::Failed {
            request: 6,
            code: ErrorCode::WrongPassword,
            message: "another password".into(),
        },
    ];
    for message in messages {
        assert_eq!(round_trip(protocol, &message).unwrap(), message);
    }
    for status in [
        UnlockStatus::Unlocked,
        UnlockStatus::WrongPassword,
        UnlockStatus::NotEncrypted,
        UnlockStatus::Unsupported,
    ] {
        let reply = Message::UnlockReply { request: 1, status };
        assert_eq!(round_trip(protocol, &reply).unwrap(), reply);
    }
}

#[test]
fn earlier_protocols_keep_their_bytes_and_refuse_protocol_four_messages() {
    let mut encrypted = node(Kind::File);
    encrypted.encryption = Encryption::ZipCrypto;
    let listing = Message::Entries {
        request: 3,
        entries: vec![encrypted],
        more: false,
    };
    for protocol in [Protocol::V2, Protocol::V3] {
        // Earlier protocols carry no encryption: it decodes as none, as before.
        let Message::Entries { entries, .. } = round_trip(protocol, &listing).unwrap() else {
            panic!("entries");
        };
        assert_eq!(entries[0].encryption, Encryption::None);
        let mut four = Vec::new();
        write_message_with_protocol(&mut four, Protocol::V4, &listing).unwrap();
        let mut three = Vec::new();
        write_message_with_protocol(&mut three, protocol, &listing).unwrap();
        assert_eq!(four.len(), three.len() + 3, "encryption, flag and reason");
        for message in [
            Message::Unlock {
                request: 1,
                password: Password::new(b"secret".to_vec()),
            },
            Message::UnlockReply {
                request: 1,
                status: UnlockStatus::Unlocked,
            },
            Message::Failed {
                request: 1,
                code: ErrorCode::PasswordRequired,
                message: String::new(),
            },
        ] {
            assert!(write_message_with_protocol(&mut Vec::new(), protocol, &message).is_err());
            let mut bytes = Vec::new();
            write_message_with_protocol(&mut bytes, Protocol::V4, &message).unwrap();
            // The same frame with an earlier version is refused.
            bytes[4..6].copy_from_slice(&(protocol as u16).to_le_bytes());
            assert!(read_message_with_protocol(&mut bytes.as_slice(), protocol).is_err());
        }
    }
}

#[test]
fn protocol_four_refuses_inconsistent_nodes_and_empty_passwords() {
    let protocol = Protocol::V4;
    let mut directory = node(Kind::Directory);
    directory.encryption = Encryption::Aes256;
    let mut reasonless = node(Kind::File);
    reasonless.readable = false;
    let mut readable_with_reason = node(Kind::File);
    readable_with_reason.unreadable = Some(Unreadable::Limit);
    let mut file_containing = node(Kind::File);
    file_containing.contains_encrypted = true;
    let mut unnamed = node(Kind::File);
    unnamed.readable = false;
    unnamed.unreadable = Some(Unreadable::Compression(String::new()));
    for invalid in [
        directory,
        reasonless,
        readable_with_reason,
        file_containing,
        unnamed,
    ] {
        let message = Message::Entries {
            request: 1,
            entries: vec![invalid],
            more: false,
        };
        assert!(write_message_with_protocol(&mut Vec::new(), protocol, &message).is_err());
    }
    let empty = Message::Unlock {
        request: 1,
        password: Password::new(Vec::new()),
    };
    assert!(write_message_with_protocol(&mut Vec::new(), protocol, &empty).is_err());
    let long = Message::Unlock {
        request: 1,
        password: Password::new(vec![b'x'; MAX_PASSWORD + 1]),
    };
    assert!(write_message_with_protocol(&mut Vec::new(), protocol, &long).is_err());
    let long = Message::Entries {
        request: 1,
        entries: vec![Node {
            readable: false,
            unreadable: Some(Unreadable::Compression("m".repeat(MAX_METHOD_NAME + 1))),
            ..node(Kind::File)
        }],
        more: false,
    };
    assert!(write_message_with_protocol(&mut Vec::new(), protocol, &long).is_err());
}

#[test]
fn a_password_never_appears_in_debug_output() {
    let message = Message::Unlock {
        request: 1,
        password: Password::new(b"hunter2".to_vec()),
    };
    let text = format!("{message:?}");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains("104"), "no byte values either: {text}");
    assert!(text.contains("<redacted>"), "{text}");
}
