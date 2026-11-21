use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::io::{Seek, SeekFrom};

#[derive(Debug, Default)]
struct Source {
    cancelled: AtomicBool,
    /// A request token cancelled at the same points as `cancelled`.
    token: Cancellation,
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
            self.token.cancel();
        }
        if count == self.stop_on_validation.load(Ordering::SeqCst) {
            // The supervisor's idle deadline ends the worker meanwhile.
            self.supervision
                .lock()
                .unwrap()
                .upgrade()
                .unwrap()
                .request_stop(Stop::Expired);
        }
        if count == self.change_on_validation.load(Ordering::SeqCst) {
            return Err(Error("fixture source changed".into()));
        }
        Ok(())
    }
    fn read_at(&self, _: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.cancelled.store(true, Ordering::Release);
        self.token.cancel();
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
fn cancelling_a_listing_discards_its_page_and_keeps_the_shared_mount() {
    let page = |request| Message::Entries {
        request,
        entries: vec![],
        more: false,
    };
    for protocol in [Protocol::V1, Protocol::V2, Protocol::V3] {
        let fixture = Fixture::new(protocol, vec![page(1), page(2)]);
        // The request token is cancelled while the page exchange is in flight.
        fixture
            .source
            .cancel_on_validation
            .store(1, Ordering::SeqCst);
        let token = fixture.source.token.clone();
        assert_eq!(
            fixture.mount.list(1, &token).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
        assert!(
            !fixture.mount.supervision.stopped(),
            "{protocol:?}: cancelled listing ended the shared worker"
        );
        // Cancellation before a page sends nothing; the next listing reads page two.
        assert!(fixture.mount.list(1, &token).is_err());
        assert!(
            fixture
                .mount
                .list(1, &Cancellation::default())
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn protocol_three_hard_read_cancellation_is_cooperative_and_keeps_the_mount() {
    let mut replies = vec![begin(1), source_read(), append(1, 1), cancelled(1)];
    replies.extend(followup(2));
    let fixture = Fixture::new(Protocol::V3, replies);
    let token = fixture.source.token.clone();
    assert_eq!(
        fixture.mount.read(2, 0, 4, &token).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert!(
        !fixture.mount.supervision.stopped(),
        "cancelled read killed shared mount"
    );
    assert!(fixture.sent().iter().any(|message| matches!(
        message,
        Message::Failed {
            request: 1,
            code: api::ErrorCode::Cancelled,
            ..
        }
    )));
    fixture.source.cancelled.store(false, Ordering::Release);
    fixture.mount.list(1, &Cancellation::default()).unwrap();
    assert_eq!(fixture.read().unwrap(), b"data");
}

#[test]
fn legacy_read_cancellation_drains_the_exchange_and_keeps_the_mount() {
    for protocol in [Protocol::V1, Protocol::V2] {
        let mut replies = if protocol == Protocol::V1 {
            vec![
                source_read(),
                Message::Content {
                    request: 1,
                    eof: true,
                    bytes: b"data".to_vec(),
                },
            ]
        } else {
            vec![
                begin(1),
                source_read(),
                append(1, 1),
                seal(1, 1),
                ready(1, 1),
            ]
        };
        replies.push(Message::Entries {
            request: 2,
            entries: vec![],
            more: false,
        });
        let fixture = Fixture::new(protocol, replies);
        // The brokered source read cancels the request token mid-exchange. These
        // protocols cannot interrupt the worker, so the host finishes the exchange and
        // discards its content instead of ending the shared worker.
        let token = fixture.source.token.clone();
        assert_eq!(
            fixture.mount.read(2, 0, 4, &token).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
        assert!(token.is_cancelled());
        assert!(
            !fixture.mount.supervision.stopped(),
            "{protocol:?}: a cancelled read ended the shared worker"
        );
        assert!(
            !fixture
                .sent()
                .iter()
                .any(|message| matches!(message, Message::Failed { .. }))
        );
        // Like a cancelled protocol-3 read, the drained member is not cached either.
        if let Some(staging) = fixture.mount.staging.lock().unwrap().as_mut() {
            assert!(staging.cached(2).is_none(), "{protocol:?}");
        }
        fixture.mount.list(1, &Cancellation::default()).unwrap();
    }
}

#[test]
fn interleaved_reads_do_not_restage_a_member_larger_than_the_cache_budget() {
    let stage = |request, entry, stage, content: &[u8]| {
        let length = content.len() as u64;
        vec![
            Message::StageBegin {
                request,
                purpose: api::StagePurpose::Member { entry },
                expected_length: Some(length),
                max_length: length,
            },
            Message::StageAppend {
                request,
                stage,
                offset: 0,
                bytes: content.to_vec(),
            },
            Message::StageSeal {
                request,
                stage,
                length,
            },
            Message::ContentReady {
                request,
                entry,
                stage,
                offset: 0,
                length,
            },
        ]
    };
    for protocol in [Protocol::V2, Protocol::V3] {
        let mut replies = stage(1, 2, 1, b"12345678");
        replies.extend(stage(2, 3, 2, b"abcd"));
        let fixture = Fixture::new(protocol, replies);
        fixture
            .mount
            .staging
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .set_cache_budget(6);
        let token = Cancellation::default();
        assert_eq!(fixture.mount.read(2, 0, 4, &token).unwrap(), b"1234");
        assert_eq!(fixture.mount.read(3, 0, 4, &token).unwrap(), b"abcd");
        // The worker has no further replies, so any re-stage would fail these reads.
        for _ in 0..8 {
            assert_eq!(fixture.mount.read(2, 4, 4, &token).unwrap(), b"5678");
            assert_eq!(
                fixture.mount.read_cooperative(3, 0, 4, &|| false).unwrap(),
                b"abcd"
            );
        }
        let requests = fixture
            .sent()
            .iter()
            .filter(|message| matches!(message, Message::Read { .. }))
            .count();
        assert_eq!(
            requests, 2,
            "{protocol:?}: a cached member was staged again"
        );
        assert!(!fixture.mount.supervision.stopped());
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

/// A scripted worker has no process for its supervisor to end.
struct ScriptedWorker;
impl close::SupervisorProcess for ScriptedWorker {
    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn wait_terminated(&mut self) -> io::Result<()> {
        Ok(())
    }
}
thread_local! {
    /// Queries of `volume_filling_up` on this test's thread.
    static VOLUME_QUERIES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
/// A 100 GiB spool volume whose 2 GiB reserve has four bytes more free at the first
/// query and none at later ones, as when another program fills it meanwhile.
fn volume_filling_up(
    _: &midnight_worker_platform::spool::PrivateSpoolDirectory,
) -> io::Result<midnight_worker_platform::spool::SpoolSpace> {
    let first = VOLUME_QUERIES.replace(VOLUME_QUERIES.get() + 1) == 0;
    Ok(midnight_worker_platform::spool::SpoolSpace {
        available: (2 << 30) + if first { 4 } else { 0 },
        total: 100 << 30,
    })
}

#[test]
fn a_member_refused_at_the_spool_reserve_ends_the_worker_and_teardown_frees_its_stage() {
    // Content whose length the worker does not declare meets the reserve as it is
    // appended: its first chunk fits, the second no longer does. (A declared length is
    // refused before anything is staged: `nested_session_tests`.)
    let undeclared = Message::StageBegin {
        request: 1,
        purpose: api::StagePurpose::Member { entry: 2 },
        expected_length: None,
        max_length: 8,
    };
    let second = Message::StageAppend {
        request: 1,
        stage: 1,
        offset: 4,
        bytes: b"more".to_vec(),
    };
    let fixture = Fixture::new(Protocol::V3, vec![undeclared, append(1, 1), second]);
    {
        let mut staging = fixture.mount.staging.lock().unwrap();
        let staging = staging.as_mut().unwrap();
        // Only content in a spool file meets the volume's reserve.
        staging.spool_files_only();
        staging.set_space(volume_filling_up);
    }
    let error = fixture
        .mount
        .read(2, 0, 4, &Cancellation::default())
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::StorageFull);
    assert!(error.message.contains("free-space reserve"), "{error}");
    // Like any other staging failure, the refusal ends the shared worker, whose
    // supervisor then releases the staging.
    assert!(fixture.mount.supervision.stopped());
    // Windows lists an open spool as a delete-on-close file; Linux spools are anonymous.
    let spool = fixture._temp.path().join("spool");
    let spools = || std::fs::read_dir(&spool).unwrap().count();
    assert_eq!(spools(), usize::from(cfg!(windows)));
    let accounting = Arc::new(AtomicUsize::new(0));
    close::supervise(
        close::Supervisor::new(
            ScriptedWorker,
            fixture.mount.staging.clone(),
            close::Accounting::for_test(accounting.clone()),
        ),
        fixture.mount.supervision.clone(),
        fixture.mount.close.clone(),
    );
    // The supervisor's teardown dropped the staging with its partial stage.
    assert!(fixture.mount.supervisor_closed_within(Duration::ZERO));
    assert!(!fixture.mount.staging_present());
    assert_eq!(spools(), 0);
    assert_eq!(accounting.load(Ordering::SeqCst), 0);
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
    // The cancelled request itself still sees its cancellation.
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
    let error = fixture.mount.list(1, &Cancellation::default()).unwrap_err();
    // Later requests are told that the watchdog ended the worker, not "cancelled", and
    // are not retryable.
    assert_eq!(
        error.kind,
        io::ErrorKind::InvalidData,
        "a racing watchdog stop must not leave a usable mount: {error}"
    );
    assert!(error.message.contains("no progress"), "{error}");
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

fn member(id: u64, parent: u64) -> api::Node {
    api::Node {
        id,
        parent,
        name: b"m".to_vec(),
        display: "m".into(),
        kind: api::Kind::File,
        size: None,
        readable: true,
        modified: None,
        encryption: api::Encryption::None,
        contains_encrypted: false,
        unreadable: None,
    }
}
/// The conforming root page that a worker which kept running would serve to `request`.
fn next_page(request: u64) -> Message {
    Message::Entries {
        request,
        entries: vec![member(2, 1)],
        more: false,
    }
}
/// Replies to one listing of these pages, then to the next request.
fn listing_then_next(pages: Vec<(Vec<api::Node>, bool)>) -> Vec<Message> {
    let mut replies: Vec<Message> = (1..)
        .zip(pages)
        .map(|(request, (entries, more))| Message::Entries {
            request,
            entries,
            more,
        })
        .collect();
    replies.push(next_page(replies.len() as u64 + 1));
    replies
}
/// A protocol violation ends the worker: later requests fail without an exchange, as
/// after any other failed exchange, and report that failure instead of a cancellation,
/// as not retryable.
fn assert_worker_ended(fixture: &Fixture, context: &str) {
    assert!(
        fixture.mount.supervision.stopped(),
        "{context}: the violating worker kept serving the mount"
    );
    let error = fixture.mount.list(1, &Cancellation::default()).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidData, "{context}: {error}");
    assert!(
        error.message.contains("stopped after a failed request"),
        "{context}: {error}"
    );
}

#[test]
fn an_unrequested_cancellation_reply_is_a_violation_not_a_cancellation() {
    for protocol in [Protocol::V1, Protocol::V2, Protocol::V3] {
        let context = format!("{protocol:?}");
        let fixture = Fixture::new(protocol, vec![cancelled(1), next_page(2)]);
        let error = fixture
            .mount
            .read(2, 0, 4, &Cancellation::default())
            .unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::InvalidData, "{context}: {error}");
        assert!(
            error
                .message
                .contains("unrequested cancellation (request canceled)"),
            "{context}: {error}"
        );
        assert_worker_ended(&fixture, &context);
        // The later error names the violation that ended the worker.
        let later = fixture
            .mount
            .read(2, 0, 4, &Cancellation::default())
            .unwrap_err();
        assert!(
            later.message.contains("unrequested cancellation"),
            "{context}: {later}"
        );
        assert!(
            !fixture.sent().iter().any(|message| matches!(
                message,
                Message::Read { request: 2, .. } | Message::List { .. }
            )),
            "{context}: a stopped worker received another request"
        );
    }
}

#[test]
fn listing_pages_that_break_wire_rules_end_the_shared_worker() {
    let every = [Protocol::V1, Protocol::V2, Protocol::V3];
    // Protocols 2 and 3 reject a member ID below 2 or a member that is its own parent
    // when they decode the frame, which already fails the exchange.
    let legacy = [Protocol::V1];
    // Each case breaks one rule: (case, protocols, listed directory, worker pages).
    type Case<'a> = (&'a str, &'a [Protocol], u64, Vec<(Vec<api::Node>, bool)>);
    let cases: [Case; 6] = [
        // Only a final page may be empty; this one would never advance the cursor.
        (
            "empty page announcing more",
            &every,
            1,
            vec![(vec![], true)],
        ),
        // The root is 1; member IDs begin at 2.
        (
            "member ID below 2",
            &legacy,
            5,
            vec![(vec![member(1, 5)], false)],
        ),
        (
            "repeated member ID",
            &every,
            1,
            vec![(vec![member(3, 1), member(3, 1)], false)],
        ),
        // IDs also increase past the cursor that the next page request carries.
        (
            "member ID at the page cursor",
            &every,
            1,
            vec![(vec![member(4, 1)], true), (vec![member(4, 1)], false)],
        ),
        (
            "directory listing itself",
            &legacy,
            5,
            vec![(vec![member(5, 5)], false)],
        ),
        (
            "member of another parent",
            &every,
            1,
            vec![(vec![member(2, 3)], false)],
        ),
    ];
    for (case, protocols, directory, pages) in cases {
        for &protocol in protocols {
            let context = format!("{protocol:?}, {case}");
            let fixture = Fixture::new(protocol, listing_then_next(pages.clone()));
            assert_eq!(
                fixture
                    .mount
                    .list(directory, &Cancellation::default())
                    .unwrap_err()
                    .kind,
                io::ErrorKind::InvalidData,
                "{context}"
            );
            assert_worker_ended(&fixture, &context);
        }
    }
}

#[test]
fn cancelling_a_listing_does_not_hide_a_page_that_breaks_wire_rules() {
    for protocol in [Protocol::V1, Protocol::V2, Protocol::V3] {
        let fixture = Fixture::new(
            protocol,
            listing_then_next(vec![(vec![member(2, 3)], false)]),
        );
        // The request token is cancelled while the page exchange is in flight. The
        // violation, not the cancellation, decides the result.
        fixture
            .source
            .cancel_on_validation
            .store(1, Ordering::SeqCst);
        assert_eq!(
            fixture
                .mount
                .list(1, &fixture.source.token)
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidData,
            "{protocol:?}"
        );
        assert!(fixture.source.token.is_cancelled(), "{protocol:?}");
        assert_worker_ended(&fixture, &format!("{protocol:?}"));
    }
}

#[test]
fn protocol_one_content_longer_than_requested_ends_the_shared_worker() {
    let oversized = Message::Content {
        request: 1,
        eof: true,
        bytes: b"data!".to_vec(),
    };
    let fixture = Fixture::new(Protocol::V1, vec![oversized.clone(), next_page(2)]);
    assert_eq!(
        fixture
            .mount
            .read(2, 0, 4, &Cancellation::default())
            .unwrap_err()
            .kind,
        io::ErrorKind::InvalidData
    );
    assert_worker_ended(&fixture, "read");
    // The brokered source read cancels this read mid-exchange, which hides nothing.
    let fixture = Fixture::new(Protocol::V1, vec![source_read(), oversized, next_page(2)]);
    assert_eq!(
        fixture.read().unwrap_err().kind,
        io::ErrorKind::InvalidData,
        "cancelled read"
    );
    assert!(fixture.source.cancelled.load(Ordering::Acquire));
    assert_worker_ended(&fixture, "cancelled read");
}

#[test]
fn a_worker_that_ends_by_itself_is_reported_with_its_memory() {
    // No reply at all: the worker's pipe closed, as when an archive whose index
    // outgrows the worker's memory ends it.
    for (protocol, memory) in [(Protocol::V1, "512 MiB"), (Protocol::V3, "1024 MiB")] {
        let fixture = Fixture::new(protocol, Vec::new());
        let error = fixture.mount.list(1, &Cancellation::default()).unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::UnexpectedEof, "{protocol:?}");
        assert_eq!(
            error.message,
            format!(
                "The archive worker ended unexpectedly, for example because the archive \
                 needs more than the {memory} of memory that the worker may use"
            ),
            "{protocol:?}"
        );
        assert_worker_ended(&fixture, &format!("{protocol:?}"));
    }
}

#[test]
fn a_folder_beyond_the_former_listing_limits_lists_completely_across_pages() {
    // The host once refused a folder of more than 100,000 members or 8 MiB of raw and
    // display names. It pages the same way on every protocol, so the costly member
    // count case (hundreds of pages) runs on protocol 3 only.
    for (case, protocols, count, name_bytes, per_page) in [
        ("members", &[Protocol::V3][..], 100_001, 1, api::MAX_BATCH),
        // Maximal raw and display names fit the 1 MiB frame budget seven to a page.
        (
            "name bytes",
            &[Protocol::V1, Protocol::V2, Protocol::V3][..],
            8 * 1024 * 1024 / (2 * api::MAX_NAME) + 1,
            api::MAX_NAME,
            7,
        ),
    ] {
        let members: Vec<api::Node> = (2..)
            .take(count)
            .map(|id| api::Node {
                name: vec![b'n'; name_bytes],
                display: "d".repeat(name_bytes),
                ..member(id, 1)
            })
            .collect();
        let chunks: Vec<&[api::Node]> = members.chunks(per_page).collect();
        for &protocol in protocols {
            let context = format!("{protocol:?}, {case}");
            let pages = chunks
                .iter()
                .enumerate()
                .map(|(index, chunk)| (chunk.to_vec(), index + 1 < chunks.len()))
                .collect();
            let fixture = Fixture::new(protocol, listing_then_next(pages));
            let listed = fixture.mount.list(1, &Cancellation::default()).unwrap();
            assert_eq!(listed.len(), count, "{context}");
            assert!(
                listed.iter().zip(&members).all(|(listed, member)| {
                    listed.id.entry == member.id
                        && *listed.raw_name == *member.name
                        && listed.display_name == member.display
                }),
                "{context}"
            );
            // Each page was asked for once, after the last member of the page before.
            let asked: Vec<u64> = fixture
                .sent()
                .iter()
                .filter_map(|message| match message {
                    Message::List { after, .. } => Some(*after),
                    _ => None,
                })
                .collect();
            let cursors: Vec<u64> = std::iter::once(0)
                .chain(chunks.iter().map(|chunk| chunk[chunk.len() - 1].id))
                .take(chunks.len())
                .collect();
            assert_eq!(asked, cursors, "{context}");
            // The worker serves the next listing of the mount.
            assert!(!fixture.mount.supervision.stopped(), "{context}");
            assert_eq!(
                fixture
                    .mount
                    .list(1, &Cancellation::default())
                    .unwrap()
                    .len(),
                1,
                "{context}"
            );
        }
    }
}

fn staged_read(request: u64) -> Vec<Message> {
    vec![
        begin(request),
        append(request, 1),
        seal(request, 1),
        ready(request, 1),
    ]
}

/// A source that fails inside an exchange (here its revalidation, as for an unreachable
/// file) stops the worker. That failure keeps its own kind, but every later request
/// through the mount reports the stopped worker as not retryable, so a Copy offers Skip
/// and Cancel instead of a Retry that could never succeed, and says to open it again.
#[test]
fn requests_after_a_failure_that_stopped_the_worker_are_not_retryable() {
    let fixture = Fixture::new(Protocol::V3, staged_read(1));
    fixture
        .source
        .change_on_validation
        .store(1, Ordering::SeqCst);
    let failure = fixture.read().unwrap_err();
    assert_eq!(failure.kind, io::ErrorKind::Other, "{failure}");
    assert!(fixture.mount.supervision.stopped());
    let later = fixture.read().unwrap_err();
    assert_eq!(later.kind, io::ErrorKind::InvalidData, "{later}");
    assert!(
        later.message.contains("fixture source changed")
            && later.message.contains("open the archive again"),
        "{later}"
    );
    assert!(
        !fixture
            .sent()
            .iter()
            .any(|message| matches!(message, Message::Read { .. })),
        "a stopped worker received a request"
    );
}

/// The same failure outside an exchange, in a revalidation between requests (Copy
/// revalidates before every chunk), leaves the worker running: it stays retryable, and
/// the mount reads on once the source is back.
#[test]
fn a_failure_the_worker_survives_stays_retryable() {
    let fixture = Fixture::new(Protocol::V3, staged_read(1));
    fixture
        .source
        .change_on_validation
        .store(1, Ordering::SeqCst);
    let failure = fixture.mount.validate().unwrap_err();
    assert_eq!(failure.kind, io::ErrorKind::Other, "{failure}");
    assert!(!fixture.mount.supervision.stopped());
    assert_eq!(fixture.read().unwrap(), b"data");
}
