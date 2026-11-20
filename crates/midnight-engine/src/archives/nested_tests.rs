//! Opening an archive inside an archive: the engine's refusals before any staging.
use super::*;

/// A mounted archive: it never stages here, since every request below is refused first.
#[derive(Debug)]
struct Enclosing(u64, Option<ArchiveParent>);
impl ArchiveMount for Enclosing {
    fn id(&self) -> u64 {
        self.0
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Ok(Vec::new())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        unreachable!("a refused open reads nothing")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
    fn parent(&self) -> Option<ArchiveParent> {
        self.1.clone()
    }
    fn stage_member(
        &self,
        _: u64,
        _: &dyn Fn() -> bool,
        _: &dyn Fn(u64),
    ) -> Result<Arc<dyn ArchiveContent>, FsError> {
        panic!("a refused open staged its member")
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        Ok(())
    }
}

/// `inner.zip` (member 9) in the root of an archive `depth` levels deep.
fn member_of(depth: u64) -> ArchiveParent {
    let mut parent: Option<ArchiveParent> = None;
    for level in 1..=depth {
        let location = ArchiveLocation {
            mount: Some(Arc::new(Enclosing(100 + level, parent.take()))),
            address: ArchiveAddress {
                provider: "midnight.fs.archive".into(),
                format: Some(ArchiveFormat::Zip),
                container: std::env::temp_dir().join("outer.zip"),
                source_version: vec![1],
                components: Vec::new(),
            },
            directory: 1,
            trail: vec![(1, "outer.zip".into())],
        };
        parent = Some(ArchiveParent {
            member: VirtualEntryId {
                mount: 100 + level,
                entry: 9,
            },
            location,
            name: "inner.zip".into(),
        });
    }
    parent.unwrap()
}

/// The error with which the engine answers `action`.
async fn refused(action: ArchiveAction) -> FsError {
    let snapshots: Snapshots = crate::test_snapshots(Default::default());
    let (sender, mut events) = tokio::sync::mpsc::channel(4);
    let mut controller = Controller::testing(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        snapshots,
        Arc::new(Semaphore::new(2)),
    );
    let mut tasks = JoinSet::new();
    controller
        .navigate(
            PaneId::Left,
            NavigationId(2),
            action,
            SortSpec::default(),
            &mut tasks,
        )
        .await;
    let error = match events.recv().await {
        Some(Event::ArchiveListed {
            result: Err(error), ..
        }) => error,
        _ => panic!("a refused open of an archive inside an archive"),
    };
    tasks.join_next().await.unwrap().unwrap();
    error
}

#[tokio::test]
async fn archives_inside_archives_need_their_session_depth_and_provider() {
    let open = |parent| ArchiveAction::OpenMember {
        parent,
        format: ArchiveFormat::Zip,
    };
    let separator = std::path::MAIN_SEPARATOR;
    let mut closed = member_of(1);
    closed.location.mount = None;
    let error = refused(open(closed)).await;
    assert!(error.message.contains("no longer open"), "{error}");
    assert!(
        error
            .path
            .to_string_lossy()
            .ends_with(&format!("outer.zip{separator}inner.zip")),
        "the error names the member: {error}"
    );
    let error = refused(open(member_of(MAX_ARCHIVE_DEPTH as u64))).await;
    assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
    assert!(error.message.contains("at most 4 levels deep"), "{error}");
    let mut foreign = member_of(1);
    foreign.member.mount = 7;
    let error = refused(open(foreign)).await;
    assert_eq!(error.kind, std::io::ErrorKind::InvalidInput);
    // No provider is installed in the test catalog: refused before any staging.
    let error = refused(open(member_of(2))).await;
    assert!(
        error
            .message
            .contains("Compatible archive filesystem plugin is unavailable"),
        "{error}"
    );
    assert!(
        error.path.to_string_lossy().ends_with(&format!(
            "outer.zip{separator}inner.zip{separator}inner.zip"
        )),
        "{error}"
    );
}
