//! Staging a member for an archive inside an archive, against scripted worker replies.
use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::io::{Seek, SeekFrom};

#[derive(Debug, Default)]
struct Source;
impl ContentSource for Source {
    fn length(&self) -> u64 {
        64
    }
    fn display_name(&self) -> String {
        "outer.zip".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![7]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, length: u32) -> Result<Vec<u8>, Error> {
        Ok(vec![0; length as usize])
    }
}

struct Fixture {
    mount: Mount,
    _temp: tempfile::TempDir,
}
impl Fixture {
    /// A mount whose worker answers with `replies`, staging member files on disk.
    fn new(protocol: Protocol, replies: Vec<Message>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut output = tempfile::tempfile().unwrap();
        for reply in replies {
            api::write_message_with_protocol(&mut output, protocol, &reply).unwrap();
        }
        output.rewind().unwrap();
        let mut staging = crate::staging::Staging::open(&temp.path().join("spool")).unwrap();
        staging.spool_files_only();
        let mount = Mount {
            id: 5,
            provider: "fixture".into(),
            provider_version: "1".into(),
            stable_entry_ids: true,
            format: Format::Zip,
            container: temp.path().join("outer.zip"),
            source: Some(CapturedFile::from_source(Arc::new(Source)).unwrap()),
            channel: Mutex::new(Channel {
                input: tempfile::tempfile().unwrap(),
                output,
                request: 0,
            }),
            supervision: Arc::new(Supervision::new(None)),
            protocol,
            staging: Arc::new(Mutex::new(Some(staging))),
            close: Arc::new(close::CloseReceipt::new()),
            nested: None,
        };
        Self { mount, _temp: temp }
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

/// The worker's replies that stage member 2 as `chunks` in `stage` for `request`.
fn staged(request: u64, stage: u64, chunks: &[&[u8]]) -> Vec<Message> {
    let length = chunks.iter().map(|chunk| chunk.len() as u64).sum();
    let mut replies = vec![Message::StageBegin {
        request,
        purpose: api::StagePurpose::Member { entry: 2 },
        expected_length: Some(length),
        max_length: length,
    }];
    let mut offset = 0;
    for chunk in chunks {
        replies.push(Message::StageAppend {
            request,
            stage,
            offset,
            bytes: chunk.to_vec(),
        });
        offset += chunk.len() as u64;
    }
    replies.extend([
        Message::StageSeal {
            request,
            stage,
            length,
        },
        Message::ContentReady {
            request,
            entry: 2,
            stage,
            offset: 0,
            length,
        },
    ]);
    replies
}

#[test]
fn a_staged_member_reports_progress_and_outlives_the_cache_and_the_session() {
    let fixture = Fixture::new(Protocol::V3, staged(1, 1, &[b"inner ", b"archive"]));
    let progress = Mutex::new(Vec::new());
    let content = fixture
        .mount
        .stage_member(2, &|| false, &|staged| {
            progress.lock().unwrap().push(staged)
        })
        .unwrap();
    assert_eq!(*progress.lock().unwrap(), [6, 13]);
    assert_eq!(content.length(), 13);
    assert_eq!(content.read_at(0, 64).unwrap(), b"inner archive");
    assert_eq!(content.read_at(6, 3).unwrap(), b"arc");
    assert!(content.read_at(13, 1).unwrap().is_empty());
    // The cache already holds the member: no second decode, and no worker exchange.
    let again = fixture.mount.stage_member(2, &|| false, &|_| {}).unwrap();
    assert_eq!(again.read_at(0, 5).unwrap(), b"inner");
    assert_eq!(
        fixture
            .sent()
            .iter()
            .filter(|message| matches!(message, Message::Read { .. }))
            .count(),
        1
    );
    // The supervisor drops the whole staging (cache, spool root) when the worker ends.
    drop(fixture.mount.staging.lock().unwrap().take());
    assert_eq!(content.read_at(0, 13).unwrap(), b"inner archive");
    drop(fixture);
    assert_eq!(again.read_at(6, 7).unwrap(), b"archive");
}

#[test]
fn staging_refuses_legacy_workers_and_requests_cancelled_beforehand() {
    let legacy = Fixture::new(Protocol::V1, Vec::new());
    let error = legacy
        .mount
        .stage_member(2, &|| false, &|_| {})
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert!(error.message.contains("newer archive plugin"), "{error}");
    let current = Fixture::new(Protocol::V3, Vec::new());
    let error = current
        .mount
        .stage_member(2, &|| true, &|_| {})
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
    assert!(
        current.sent().is_empty(),
        "a cancelled stage reached the worker"
    );
    assert_eq!(
        current
            .mount
            .stage_member(1, &|| false, &|_| {})
            .unwrap_err()
            .kind,
        io::ErrorKind::InvalidInput,
        "the root is no member"
    );
}

#[test]
fn a_cancelled_stage_keeps_the_shared_worker_for_the_next_one() {
    let mut replies = staged(1, 1, &[b"in", b"ner"]);
    // The host cancels at the second append; the worker acknowledges.
    replies.truncate(3);
    replies.push(Message::Failed {
        request: 1,
        code: api::ErrorCode::Cancelled,
        message: "request canceled".into(),
    });
    replies.extend(staged(2, 2, &[b"inner"]));
    let fixture = Fixture::new(Protocol::V3, replies);
    let cancelled = AtomicBool::new(false);
    let error = fixture
        .mount
        .stage_member(2, &|| cancelled.load(Ordering::Acquire), &|_| {
            cancelled.store(true, Ordering::Release)
        })
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
    assert!(!fixture.mount.supervision.stopped(), "the worker was ended");
    assert!(fixture.sent().iter().any(|message| matches!(
        message,
        Message::Failed {
            code: api::ErrorCode::Cancelled,
            ..
        }
    )));
    let content = fixture.mount.stage_member(2, &|| false, &|_| {}).unwrap();
    assert_eq!(content.read_at(0, 8).unwrap(), b"inner");
}

fn package() -> Package {
    Package {
        id: "midnight.fs.archive".into(),
        name: None,
        directory: PathBuf::from("/nonexistent/archive"),
        version: "1".into(),
        stable_entry_ids: true,
        executable: PathBuf::from("/nonexistent/archive/worker"),
        protocol: Protocol::V3,
        formats: vec![Format::Zip],
    }
}

/// A mounted archive the nested open only names; `parent` makes it nested itself.
#[derive(Debug)]
struct Named(u64, Option<ArchiveParent>);
impl ArchiveMount for Named {
    fn id(&self) -> u64 {
        self.0
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        unreachable!()
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        unreachable!()
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
    fn parent(&self) -> Option<ArchiveParent> {
        self.1.clone()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        Ok(())
    }
}

/// `inner.zip` (member 9) in the root of an archive `depth` levels deep.
fn member_of(depth: u64, mount: Option<u64>) -> ArchiveParent {
    let mut parent: Option<ArchiveParent> = None;
    for level in 1..=depth {
        let location = midnight_core::ArchiveLocation {
            mount: Some(Arc::new(Named(level, parent.take()))),
            address: midnight_core::ArchiveAddress {
                provider: "midnight.fs.archive".into(),
                format: Some(midnight_core::ArchiveFormat::Zip),
                container: std::env::temp_dir().join("outer.zip"),
                source_version: vec![1],
                components: Vec::new(),
            },
            directory: 1,
            trail: vec![(1, if level == 1 { "outer.zip" } else { "inner.zip" }.into())],
        };
        parent = Some(ArchiveParent {
            member: VirtualEntryId {
                mount: mount.unwrap_or(level),
                entry: 9,
            },
            location,
            name: "inner.zip".into(),
        });
    }
    parent.unwrap()
}

#[derive(Debug)]
struct Bytes;
impl ArchiveContent for Bytes {
    fn length(&self) -> u64 {
        4
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, FsError> {
        Ok(b"PK\x05\x06".to_vec())
    }
}

#[test]
fn nested_opens_refuse_the_deepest_level_closed_parents_and_foreign_members() {
    let open = |parent| {
        Mount::open_nested(
            &package(),
            Format::Zip,
            parent,
            Arc::new(Bytes),
            &Cancellation::default(),
        )
        .unwrap_err()
    };
    let deepest = open(member_of(MAX_ARCHIVE_DEPTH as u64, None));
    assert_eq!(deepest.kind, io::ErrorKind::Unsupported);
    assert!(
        deepest.message.contains("at most 4 levels deep"),
        "{deepest}"
    );
    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        deepest
            .path
            .to_string_lossy()
            .ends_with(&format!("outer.zip{separator}inner.zip{separator}inner.zip{separator}inner.zip{separator}inner.zip")),
        "{deepest}"
    );
    let mut closed = member_of(1, None);
    closed.location.mount = None;
    assert!(open(closed).message.contains("is closed"));
    let foreign = open(member_of(1, Some(77)));
    assert_eq!(foreign.kind, io::ErrorKind::InvalidInput);
}
