use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::io::{Seek, SeekFrom};

struct CompletionSource {
    supervision: Arc<Supervision>,
    validations: AtomicUsize,
    stop_on: usize,
    cancel: Option<Cancellation>,
}

impl std::fmt::Debug for CompletionSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompletionSource")
            .finish_non_exhaustive()
    }
}

impl ContentSource for CompletionSource {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "completion fixture".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        if self.validations.fetch_add(1, Ordering::SeqCst) + 1 == self.stop_on {
            if let Some(cancel) = &self.cancel {
                cancel.cancel();
            } else {
                self.supervision.stop.store(true, Ordering::Release);
            }
        }
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        panic!("completion fixtures do not read source payload")
    }
}

fn operation() -> Message {
    Message::Read {
        request: 1,
        entry: 2,
        offset: 0,
        length: 4,
    }
}

fn staged_replies() -> Vec<Message> {
    vec![
        Message::StageBegin {
            request: 1,
            purpose: api::StagePurpose::Member { entry: 2 },
            expected_length: Some(4),
            max_length: 4,
        },
        Message::StageAppend {
            request: 1,
            stage: 1,
            offset: 0,
            bytes: b"data".to_vec(),
        },
        Message::StageSeal {
            request: 1,
            stage: 1,
            length: 4,
        },
        Message::ContentReady {
            request: 1,
            entry: 2,
            stage: 1,
            offset: 0,
            length: 4,
        },
    ]
}

fn verify_completion(
    protocol: Protocol,
    cached: bool,
    listing: bool,
    stop_on: usize,
    cancelled: bool,
) {
    let temporary = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let supervision = Arc::new(Supervision {
        stop: AtomicBool::new(false),
        deadline: Mutex::new(None),
    });
    let source = Arc::new(CompletionSource {
        supervision: supervision.clone(),
        validations: AtomicUsize::new(0),
        stop_on,
        cancel: cancelled.then(|| cancel.clone()),
    });
    let captured = CapturedFile::from_source(source.clone()).unwrap();
    source.validations.store(0, Ordering::SeqCst);
    let mut output = tempfile::tempfile().unwrap();
    let mut staging = (protocol == Protocol::V2)
        .then(|| crate::staging::Staging::open(&temporary.path().join("private")).unwrap());
    if cached {
        let state = staging.as_mut().unwrap();
        let replies = staged_replies();
        for message in &replies[..3] {
            state.nested(&operation(), message).unwrap();
        }
        state.terminal(&operation(), &replies[3]).unwrap();
    } else {
        let replies = if listing {
            vec![Message::Entries {
                request: 1,
                entries: vec![],
                more: false,
            }]
        } else if protocol == Protocol::V2 {
            staged_replies()
        } else {
            vec![Message::Content {
                request: 1,
                bytes: b"data".to_vec(),
                eof: true,
            }]
        };
        for reply in replies {
            api::write_message_with_protocol(&mut output, protocol, &reply).unwrap();
        }
        output.seek(SeekFrom::Start(0)).unwrap();
    }
    let mount = Mount {
        id: 1,
        provider: "fixture".into(),
        provider_version: "1".into(),
        stable_entry_ids: false,
        format: Format::Zip,
        container: temporary.path().join("archive.zip"),
        source: captured,
        channel: Mutex::new(Channel {
            input: tempfile::tempfile().unwrap(),
            output,
            request: 0,
        }),
        supervision,
        protocol,
        staging: Arc::new(Mutex::new(staging)),
    };
    let result = if listing {
        mount.list(1, &cancel).map(|nodes| nodes.len())
    } else {
        mount.read(2, 0, 4, &cancel).map(|bytes| bytes.len())
    };
    assert!(source.validations.load(Ordering::SeqCst) >= stop_on);
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::Interrupted);
    if protocol == Protocol::V2 && !cached && !listing && stop_on == 2 {
        assert!(
            mount
                .staging
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .cached(2)
                .is_none(),
            "a stopped exchange must not publish newly staged content"
        );
    }
}

#[test]
fn stopped_completion_never_publishes_content_or_metadata() {
    for (protocol, cached, listing, stop_on) in [
        (Protocol::V2, true, false, 2),
        (Protocol::V2, false, false, 2),
        (Protocol::V2, false, false, 3),
        (Protocol::V1, false, false, 2),
        (Protocol::V2, false, true, 2),
        (Protocol::V1, false, true, 2),
    ] {
        verify_completion(protocol, cached, listing, stop_on, false);
    }
}

#[test]
fn cancelled_completion_never_publishes_content_or_metadata() {
    for (protocol, cached, listing, stop_on) in [
        (Protocol::V2, true, false, 2),
        (Protocol::V2, false, false, 2),
        (Protocol::V2, false, false, 3),
        (Protocol::V1, false, false, 2),
        (Protocol::V2, false, true, 2),
        (Protocol::V1, false, true, 2),
    ] {
        verify_completion(protocol, cached, listing, stop_on, true);
    }
}
