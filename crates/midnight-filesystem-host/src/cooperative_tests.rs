use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::io::{Seek, SeekFrom};

#[derive(Debug, Default)]
struct Source {
    cancelled: AtomicBool,
    validations: AtomicUsize,
    cancel_on_validation: AtomicUsize,
    change_on_validation: AtomicUsize,
    stop_on_validation: AtomicUsize,
    supervision: Mutex<std::sync::Weak<Supervision>>,
}
impl ContentSource for Source {
    fn length(&self) -> u64 {
        4
    }
    fn display_name(&self) -> String {
        "owned cancellation fixture".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        let count = self.validations.fetch_add(1, Ordering::SeqCst) + 1;
        if count == self.cancel_on_validation.load(Ordering::SeqCst) {
            self.cancelled.store(true, Ordering::Release);
        }
        if count == self.stop_on_validation.load(Ordering::SeqCst) {
            self.supervision
                .lock()
                .unwrap()
                .upgrade()
                .unwrap()
                .stop
                .store(true, Ordering::Release);
        }
        if count == self.change_on_validation.load(Ordering::SeqCst) {
            return Err(Error("fixture source changed".into()));
        }
        Ok(())
    }
    fn read_at(&self, _: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.cancelled.store(true, Ordering::Release);
        Ok(vec![7; length as usize])
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
fn append(request: u64, stage: u64) -> Message {
    Message::StageAppend {
        request,
        stage,
        offset: 0,
        bytes: b"data".to_vec(),
    }
}
fn seal(request: u64, stage: u64) -> Message {
    Message::StageSeal {
        request,
        stage,
        length: 4,
    }
}
fn ready(request: u64, stage: u64) -> Message {
    Message::ContentReady {
        request,
        entry: 2,
        stage,
        offset: 0,
        length: 4,
    }
}
fn cancelled(request: u64) -> Message {
    Message::Failed {
        request,
        code: api::ErrorCode::Cancelled,
        message: "request canceled".into(),
    }
}
struct Fixture {
    mount: Mount,
    source: Arc<Source>,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new(protocol: Protocol, replies: Vec<Message>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = Arc::new(Source::default());
        let captured = CapturedFile::from_source(source.clone()).unwrap();
        source.validations.store(0, Ordering::SeqCst);
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
            source: captured,
            channel: Mutex::new(Channel {
                input: tempfile::tempfile().unwrap(),
                output,
                request: 0,
            }),
            supervision: Arc::new(Supervision {
                stop: AtomicBool::new(false),
                deadline: Mutex::new(None),
            }),
            protocol,
            staging: Arc::new(Mutex::new(Some(
                crate::staging::Staging::open(&temp.path().join("spool")).unwrap(),
            ))),
        };
        *source.supervision.lock().unwrap() = Arc::downgrade(&mount.supervision);
        Self {
            mount,
            source,
            _temp: temp,
        }
    }
    fn read(&self) -> Result<Vec<u8>, FsError> {
        self.mount
            .read_cooperative(2, 0, 4, &|| self.source.cancelled.load(Ordering::Acquire))
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
fn source_read() -> Message {
    Message::SourceRead {
        request: 1,
        offset: 0,
        length: 1,
    }
}
fn followup(stage: u64) -> Vec<Message> {
    vec![
        Message::Entries {
            request: 2,
            entries: vec![],
            more: false,
        },
        begin(3),
        append(3, stage),
        seal(3, stage),
        ready(3, stage),
    ]
}

#[test]
fn cooperative_checkpoints_cancel_then_list_and_read_on_the_same_mount() {
    for (prefix, checkpoint, next_stage) in [
        (vec![], begin(1), 1),
        (vec![begin(1)], append(1, 1), 2),
        (
            vec![begin(1), append(1, 1)],
            Message::StageProgress {
                request: 1,
                stage: 1,
                completed: 4,
            },
            2,
        ),
        (vec![begin(1), append(1, 1)], seal(1, 1), 2),
    ] {
        let mut replies = prefix;
        replies.extend([source_read(), checkpoint, cancelled(1)]);
        replies.extend(followup(next_stage));
        let fixture = Fixture::new(Protocol::V3, replies);
        assert_eq!(fixture.read().unwrap_err().kind, io::ErrorKind::Interrupted);
        assert!(
            !fixture.mount.supervision.stop.load(Ordering::Acquire),
            "canceled request killed shared mount"
        );
        assert!(fixture.sent().iter().any(|message| matches!(
            message,
            Message::Failed {
                request: 1,
                code: api::ErrorCode::Cancelled,
                ..
            }
        )));
        assert!(
            fixture
                .mount
                .staging
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .cached(2)
                .is_none()
        );
        fixture.source.cancelled.store(false, Ordering::Release);
        fixture.mount.list(1, &Cancellation::default()).unwrap();
        assert_eq!(fixture.read().unwrap(), b"data");
    }
}

#[test]
fn cooperative_terminal_race_does_not_insert_new_cache_content() {
    let fixture = Fixture::new(
        Protocol::V3,
        vec![
            begin(1),
            append(1, 1),
            seal(1, 1),
            ready(1, 1),
            Message::Entries {
                request: 2,
                entries: vec![],
                more: false,
            },
        ],
    );
    fixture
        .source
        .cancel_on_validation
        .store(2, Ordering::SeqCst);
    assert_eq!(fixture.read().unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(
        fixture
            .mount
            .staging
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .cached(2)
            .is_none()
    );
    assert!(!fixture.mount.supervision.stop.load(Ordering::Acquire));
    fixture.source.cancelled.store(false, Ordering::Release);
    fixture.mount.list(1, &Cancellation::default()).unwrap();
}

#[test]
fn cooperative_cancellation_does_not_hide_malformed_checkpoints_or_acknowledgements() {
    for (checkpoint, terminal) in [
        (
            Message::StageAppend {
                request: 1,
                stage: 1,
                offset: 99,
                bytes: vec![1],
            },
            cancelled(1),
        ),
        (
            Message::StageAppend {
                request: 1,
                stage: 2,
                offset: 0,
                bytes: vec![1],
            },
            cancelled(1),
        ),
        (append(1, 1), cancelled(2)),
        (
            append(1, 1),
            Message::Entries {
                request: 1,
                entries: vec![],
                more: false,
            },
        ),
    ] {
        let fixture = Fixture::new(
            Protocol::V3,
            vec![begin(1), source_read(), checkpoint, terminal],
        );
        assert!(fixture.read().is_err());
        assert!(fixture.mount.supervision.stop.load(Ordering::Acquire));
    }
}

#[test]
fn cooperative_cancellation_cannot_hide_changed_source() {
    let fixture = Fixture::new(
        Protocol::V3,
        vec![begin(1), source_read(), append(1, 1), cancelled(1)],
    );
    fixture
        .source
        .change_on_validation
        .store(2, Ordering::SeqCst);
    assert!(fixture.read().is_err());
    assert!(fixture.mount.supervision.stop.load(Ordering::Acquire));
}

#[test]
fn cooperative_legacy_read_drains_without_cancelling_shared_mount() {
    let fixture = Fixture::new(
        Protocol::V2,
        vec![
            begin(1),
            source_read(),
            append(1, 1),
            seal(1, 1),
            ready(1, 1),
            Message::Entries {
                request: 2,
                entries: vec![],
                more: false,
            },
        ],
    );
    assert_eq!(fixture.read().unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(!fixture.mount.supervision.stop.load(Ordering::Acquire));
    assert!(
        !fixture
            .sent()
            .iter()
            .any(|message| matches!(message, Message::Failed { .. }))
    );
    fixture.source.cancelled.store(false, Ordering::Release);
    fixture.mount.list(1, &Cancellation::default()).unwrap();
}

#[test]
fn cooperative_cancel_preserves_cached_member_and_retained_lease() {
    let fixture = Fixture::new(
        Protocol::V3,
        vec![begin(1), source_read(), append(1, 2), cancelled(1)],
    );
    let old = {
        let mut staging = fixture.mount.staging.lock().unwrap();
        let staging = staging.as_mut().unwrap();
        let op = Message::Read {
            request: 99,
            entry: 3,
            offset: 0,
            length: 4,
        };
        staging
            .nested(
                &op,
                &Message::StageBegin {
                    request: 99,
                    purpose: api::StagePurpose::Member { entry: 3 },
                    expected_length: Some(4),
                    max_length: 4,
                },
            )
            .unwrap();
        staging.nested(&op, &append(99, 1)).unwrap();
        staging.nested(&op, &seal(99, 1)).unwrap();
        staging
            .terminal(
                &op,
                &Message::ContentReady {
                    request: 99,
                    entry: 3,
                    stage: 1,
                    offset: 0,
                    length: 4,
                },
            )
            .unwrap()
            .unwrap()
    };
    assert_eq!(fixture.read().unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(!fixture.mount.supervision.stop.load(Ordering::Acquire));
    assert_eq!(old.read(0, 4).unwrap(), b"data");
    let mut staging = fixture.mount.staging.lock().unwrap();
    let staging = staging.as_mut().unwrap();
    assert_eq!(staging.cached(3).unwrap().read(0, 4).unwrap(), b"data");
    assert!(staging.cached(2).is_none());
}

#[test]
fn cooperative_terminal_scratch_race_preserves_verified_scratch() {
    let fixture = Fixture::new(Protocol::V3, vec![ready(1, 1), ready(2, 1)]);
    {
        let mut staging = fixture.mount.staging.lock().unwrap();
        let staging = staging.as_mut().unwrap();
        let op = Message::Open {
            request: 99,
            format: Format::TarGzip,
            length: 4,
        };
        staging
            .nested(
                &op,
                &Message::StageBegin {
                    request: 99,
                    purpose: api::StagePurpose::Scratch,
                    expected_length: Some(4),
                    max_length: 4,
                },
            )
            .unwrap();
        staging.nested(&op, &append(99, 1)).unwrap();
        staging.nested(&op, &seal(99, 1)).unwrap();
        staging
            .terminal(
                &op,
                &Message::Opened {
                    request: 99,
                    root: 1,
                },
            )
            .unwrap();
    }
    fixture
        .source
        .cancel_on_validation
        .store(2, Ordering::SeqCst);
    assert_eq!(fixture.read().unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(!fixture.mount.supervision.stop.load(Ordering::Acquire));
    assert!(
        fixture
            .mount
            .staging
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .cached(2)
            .is_none()
    );
    fixture.source.cancelled.store(false, Ordering::Release);
    assert_eq!(fixture.read().unwrap(), b"data");
}

#[test]
fn cooperative_watchdog_stop_during_cancel_acknowledgement_keeps_mount_failed() {
    let fixture = Fixture::new(Protocol::V3, vec![begin(1), cancelled(1)]);
    // Start validation requests soft cancellation; checkpoint validation succeeds,
    // and terminal-ack validation simulates the watchdog winning the race.
    fixture
        .source
        .cancel_on_validation
        .store(1, Ordering::SeqCst);
    fixture.source.stop_on_validation.store(3, Ordering::SeqCst);
    assert_eq!(fixture.read().unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(fixture.sent().iter().any(|message| matches!(
        message,
        Message::Failed {
            code: api::ErrorCode::Cancelled,
            ..
        }
    )));
    assert!(fixture.mount.supervision.stop.load(Ordering::Acquire));
    fixture.source.cancelled.store(false, Ordering::Release);
    assert!(
        fixture.mount.list(1, &Cancellation::default()).is_err(),
        "a racing watchdog stop must not leave a usable mount"
    );
}

#[test]
fn cooperative_terminal_cancellation_never_hides_bad_content_identity_or_range() {
    for (entry, stage, offset, length) in [(3, 1, 0, 4), (2, 2, 0, 4), (2, 1, 1, 3), (2, 1, 0, 5)] {
        let fixture = Fixture::new(
            Protocol::V3,
            vec![
                begin(1),
                append(1, 1),
                seal(1, 1),
                Message::ContentReady {
                    request: 1,
                    entry,
                    stage,
                    offset,
                    length,
                },
            ],
        );
        fixture
            .source
            .cancel_on_validation
            .store(2, Ordering::SeqCst);
        assert_eq!(fixture.read().unwrap_err().kind, io::ErrorKind::InvalidData);
        assert!(fixture.mount.supervision.stop.load(Ordering::Acquire));
        assert!(
            fixture
                .mount
                .staging
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .cached(2)
                .is_none()
        );
    }
}
