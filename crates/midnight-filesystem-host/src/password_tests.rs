//! Protocol 4 passwords over the real wire codec: scripted worker replies.
use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::io::{Seek, SeekFrom};

#[derive(Debug)]
struct Source;
impl ContentSource for Source {
    fn length(&self) -> u64 {
        4
    }
    fn display_name(&self) -> String {
        "owned password fixture".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, length: u32) -> Result<Vec<u8>, Error> {
        Ok(vec![7; length as usize])
    }
}

struct Fixture {
    mount: Mount,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new(protocol: Protocol, replies: Vec<Message>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let captured = CapturedFile::from_source(Arc::new(Source)).unwrap();
        let mut output = tempfile::tempfile().unwrap();
        for reply in replies {
            api::write_message_with_protocol(&mut output, protocol, &reply).unwrap();
        }
        output.rewind().unwrap();
        let mount = Mount {
            id: 1,
            provider: "fixture".into(),
            provider_version: "1".into(),
            stable_entry_ids: true,
            format: Format::Zip,
            container: temp.path().join("owned.zip"),
            source: Some(captured),
            channel: Mutex::new(Channel {
                input: tempfile::tempfile().unwrap(),
                output,
                request: 0,
            }),
            supervision: Arc::new(Supervision::new(None)),
            protocol,
            staging: Arc::new(Mutex::new(Some(
                crate::staging::Staging::open(&temp.path().join("spool")).unwrap(),
            ))),
            close: Arc::new(close::CloseReceipt::new()),
            nested: None,
        };
        Self { mount, _temp: temp }
    }
    fn read(&self) -> Result<Vec<u8>, FsError> {
        self.mount.read(2, 0, 4, &Cancellation::default())
    }
    fn stopped(&self) -> bool {
        self.mount.supervision.stopped()
    }
    fn sent(&self) -> Vec<Message> {
        let mut channel = self.mount.channel.lock().unwrap();
        channel.input.rewind().unwrap();
        let mut messages = Vec::new();
        while let Ok(message) =
            api::read_message_with_protocol(&mut channel.input, self.mount.protocol)
        {
            messages.push(message);
        }
        channel.input.seek(SeekFrom::End(0)).unwrap();
        messages
    }
}

fn failed(request: u64, code: api::ErrorCode) -> Message {
    Message::Failed {
        request,
        code,
        message: "password".into(),
    }
}
fn begin(request: u64) -> Message {
    Message::StageBegin {
        request,
        purpose: api::StagePurpose::Member { entry: 2 },
        expected_length: Some(4),
        max_length: 4,
    }
}
fn staged(request: u64, stage: u64) -> Vec<Message> {
    vec![
        begin(request),
        Message::StageAppend {
            request,
            stage,
            offset: 0,
            bytes: b"data".to_vec(),
        },
        Message::StageSeal {
            request,
            stage,
            length: 4,
        },
        Message::ContentReady {
            request,
            entry: 2,
            stage,
            offset: 0,
            length: 4,
        },
    ]
}
fn password() -> midnight_core::ArchivePassword {
    midnight_core::ArchivePassword::new("tajné heslo".into())
}

#[test]
fn a_locked_member_keeps_the_session_and_reads_after_unlock() {
    let mut replies = vec![
        failed(1, api::ErrorCode::PasswordRequired),
        Message::UnlockReply {
            request: 2,
            status: api::UnlockStatus::Unlocked,
        },
    ];
    replies.extend(staged(3, 1));
    let fixture = Fixture::new(Protocol::V4, replies);
    let error = fixture.read().unwrap_err();
    assert!(
        midnight_core::is_archive_password_required(&error),
        "{error}"
    );
    assert!(!fixture.stopped(), "a locked member ended the session");
    assert!(!fixture.mount.unlocked());
    assert_eq!(
        fixture
            .mount
            .unlock(&password(), &Cancellation::default())
            .unwrap(),
        ArchiveUnlock::Unlocked
    );
    assert!(fixture.mount.unlocked());
    assert_eq!(fixture.read().unwrap(), b"data");
    let sent = fixture.sent();
    let Message::Unlock { password: sent, .. } = &sent[1] else {
        panic!("{sent:?}");
    };
    assert_eq!(sent.as_bytes(), "tajné heslo".as_bytes());
}

#[test]
fn refused_unlocks_and_a_member_with_another_password_keep_the_session_locked() {
    let mut replies = vec![
        Message::UnlockReply {
            request: 1,
            status: api::UnlockStatus::WrongPassword,
        },
        Message::UnlockReply {
            request: 2,
            status: api::UnlockStatus::NotEncrypted,
        },
        failed(3, api::ErrorCode::WrongPassword),
    ];
    replies.extend(staged(4, 1));
    let fixture = Fixture::new(Protocol::V4, replies);
    assert_eq!(
        fixture
            .mount
            .unlock(&password(), &Cancellation::default())
            .unwrap(),
        ArchiveUnlock::WrongPassword
    );
    assert_eq!(
        fixture
            .mount
            .unlock(&password(), &Cancellation::default())
            .unwrap(),
        ArchiveUnlock::NotEncrypted
    );
    assert!(!fixture.mount.unlocked());
    let error = fixture.read().unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
    assert!(!midnight_core::is_archive_password_required(&error));
    assert!(!fixture.stopped());
    assert_eq!(fixture.read().unwrap(), b"data");
}

#[test]
fn a_listing_whose_names_are_locked_keeps_the_session() {
    let replies = vec![
        failed(1, api::ErrorCode::PasswordRequired),
        Message::Entries {
            request: 2,
            entries: vec![],
            more: false,
        },
    ];
    let fixture = Fixture::new(Protocol::V4, replies);
    let error = fixture.mount.list(1, &Cancellation::default()).unwrap_err();
    assert!(midnight_core::is_archive_password_required(&error));
    assert!(!fixture.stopped());
    assert!(
        fixture
            .mount
            .list(1, &Cancellation::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_password_reply_after_a_stage_began_ends_the_session() {
    let fixture = Fixture::new(
        Protocol::V4,
        vec![begin(1), failed(1, api::ErrorCode::PasswordRequired)],
    );
    let error = fixture.read().unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidData, "{error}");
    assert!(
        fixture.stopped(),
        "a stage began: the reply breaks the protocol"
    );
}

#[test]
fn a_protocol_three_worker_refuses_unlock_without_an_exchange() {
    let fixture = Fixture::new(Protocol::V3, vec![]);
    let error = fixture
        .mount
        .unlock(&password(), &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert!(fixture.sent().is_empty());
    assert!(!fixture.stopped());
    let empty = midnight_core::ArchivePassword::new(String::new());
    let fixture = Fixture::new(Protocol::V4, vec![]);
    assert_eq!(
        fixture
            .mount
            .unlock(&empty, &Cancellation::default())
            .unwrap_err()
            .kind,
        io::ErrorKind::InvalidInput
    );
    assert!(fixture.sent().is_empty());
}

#[test]
fn listings_carry_the_reasons_and_encryption_of_protocol_four_nodes() {
    let node =
        |id, kind, encryption, contains_encrypted, unreadable: Option<api::Unreadable>| api::Node {
            id,
            parent: 1,
            name: format!("n{id}").into_bytes(),
            display: format!("n{id}"),
            kind,
            size: Some(9),
            readable: kind == api::Kind::File && unreadable.is_none(),
            modified: None,
            encryption,
            contains_encrypted,
            unreadable,
        };
    let fixture = Fixture::new(
        Protocol::V4,
        vec![Message::Entries {
            request: 1,
            entries: vec![
                node(2, api::Kind::File, api::Encryption::Aes256, false, None),
                node(
                    3,
                    api::Kind::File,
                    api::Encryption::None,
                    false,
                    Some(api::Unreadable::Compression("Implode".into())),
                ),
                node(4, api::Kind::Directory, api::Encryption::None, true, None),
            ],
            more: false,
        }],
    );
    let nodes = fixture.mount.list(1, &Cancellation::default()).unwrap();
    assert_eq!(
        nodes[0].access.encryption,
        midnight_core::ArchiveEncryption::Aes256
    );
    assert!(nodes[0].needs_password());
    assert_eq!(
        nodes[1].access.unreadable,
        Some(midnight_core::ArchiveUnreadable::Compression(
            "Implode".into()
        ))
    );
    assert!(nodes[1].unreadable_reason().contains("Implode"));
    assert!(nodes[2].access.contains_encrypted && nodes[2].needs_password());
}
