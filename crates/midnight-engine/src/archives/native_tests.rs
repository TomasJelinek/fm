//! A navigation that opens an archive from a native file takes its session's slot first
//! (`native::slot`); the session that its tab let go of hands its slot over.
use super::*;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

/// A session of the fake host. Like a host session it holds its native slot until it ends,
/// and it reports its end once that slot is released.
#[derive(Debug)]
struct Session {
    id: u64,
    slot: Option<NativeSlot>,
    ended: mpsc::Sender<u64>,
}
impl Drop for Session {
    fn drop(&mut self) {
        drop(self.slot.take());
        let _ = self.ended.send(self.id);
    }
}
impl ArchiveMount for Session {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        drop(Arc::into_inner(self));
        Ok(())
    }
    fn id(&self) -> u64 {
        self.id
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Ok(Vec::new())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("navigation reads no member")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Next {
    Opens,
    Fails,
    Panics,
}

struct Fixture {
    controller: Controller,
    events: tokio::sync::mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    allowance: Arc<NativeAllowance>,
    ended: mpsc::Receiver<u64>,
    next: Arc<Mutex<Next>>,
}
impl Fixture {
    fn new() -> Self {
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let mut controller = Controller::testing(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            crate::test_snapshots(Default::default()),
            Arc::new(Semaphore::new(MAX_WORKERS)),
        );
        let allowance = NativeAllowance::new(2);
        controller.allowance = allowance.clone();
        let (report, ended) = mpsc::channel();
        let next = Arc::new(Mutex::new(Next::Opens));
        let behaviour = next.clone();
        let ids = AtomicU64::new(1);
        controller.opener = native::Opener::Test(Arc::new(move |slot, path| {
            let slot = slot.expect("the navigation reserved a slot");
            match *behaviour.lock().unwrap() {
                Next::Opens => {}
                // Like a host open that fails, it releases the slot it was given.
                Next::Fails => {
                    drop(slot);
                    return Err(FsError::from_io(
                        "open archive",
                        path,
                        std::io::Error::other("unreadable test archive"),
                    ));
                }
                Next::Panics => panic!("test archive open unwound"),
            }
            let id = ids.fetch_add(1, Ordering::SeqCst);
            slot.bind(id, path);
            Ok(Arc::new(Session {
                id,
                slot: Some(slot),
                ended: report.clone(),
            }) as Arc<dyn ArchiveMount>)
        }));
        Self {
            controller,
            events,
            tasks: JoinSet::new(),
            allowance,
            ended,
            next,
        }
    }
    /// Start opening the saved archive `name` in `pane`; `replaces` is the session that
    /// its tab let go of.
    async fn start(&mut self, pane: PaneId, navigation: u64, name: &str, replaces: Option<u64>) {
        let action = ArchiveAction::List(ArchiveLocation::unavailable(ArchiveAddress {
            provider: "test.native".into(),
            format: Some(ArchiveFormat::SevenZip),
            container: std::env::temp_dir().join(name),
            source_version: vec![1],
            components: Vec::new(),
        }));
        self.controller
            .navigate_replacing(
                pane,
                NavigationId(navigation),
                action,
                SortSpec::default(),
                replaces,
                &mut self.tasks,
            )
            .await;
    }
    async fn open(
        &mut self,
        pane: PaneId,
        navigation: u64,
        name: &str,
        replaces: Option<u64>,
    ) -> Result<ArchiveListing, FsError> {
        self.start(pane, navigation, name, replaces).await;
        self.listed(pane, NavigationId(navigation)).await
    }
    /// Until one open waits for a replaced session; the deadline elapses only on failure.
    async fn successor_waits(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while self.allowance.successors() == 0 {
            assert!(std::time::Instant::now() < deadline, "no successor waits");
            tokio::task::yield_now().await;
        }
    }
    async fn listed(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
    ) -> Result<ArchiveListing, FsError> {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(60), self.events.recv())
                .await
                .expect("the navigation ends")
                .expect("engine events");
            if let Event::ArchiveListed {
                pane: listed,
                navigation: found,
                result,
            } = event
                && listed == pane
                && found == navigation
            {
                return result;
            }
        }
    }
    /// The session `id` ended and its slot was released.
    fn ended(&self, id: u64) {
        assert_eq!(
            self.ended.recv_timeout(Duration::from_secs(10)),
            Ok(id),
            "session {id} ended"
        );
    }
    fn listed_sessions(&self) -> Vec<u64> {
        let mut ids: Vec<_> = self
            .allowance
            .refusal()
            .sessions
            .iter()
            .map(|session| session.id)
            .collect();
        ids.sort_unstable();
        ids
    }
    /// Both panels show an archive: sessions 1 (left) and 2 (right).
    async fn both_panels_in_archives(&mut self) -> (ArchiveListing, ArchiveListing) {
        let left = self.open(PaneId::Left, 1, "a.7z", None).await.unwrap();
        let right = self.open(PaneId::Right, 1, "x.7z", None).await.unwrap();
        assert_eq!(self.listed_sessions(), [1, 2]);
        (left, right)
    }
}

fn refused(result: Result<ArchiveListing, FsError>) -> FsError {
    let error = result.expect_err("the open is refused");
    assert!(
        error.message.contains("Two archives are already open"),
        "{error}"
    );
    error
}

#[tokio::test]
async fn a_tab_that_leaves_its_archive_for_another_hands_its_slot_over() {
    let mut fixture = Fixture::new();
    let (left, right) = fixture.both_panels_in_archives().await;
    // The left tab opens another archive and lets go of its own: its rows and session.
    drop(left);
    let opened = fixture.open(PaneId::Left, 2, "b.7z", Some(1)).await;
    let opened = opened.unwrap_or_else(|error| panic!("the tab's own archive blocked: {error}"));
    fixture.ended(1);
    assert_eq!(fixture.listed_sessions(), [2, 3]);
    assert_eq!(fixture.allowance.counted(), 2);
    assert!(
        fixture.allowance.try_reserve().is_err(),
        "a third open never takes the handed-off slot"
    );
    drop((opened, right));
    while fixture.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn a_failed_open_returns_the_handed_off_slot() {
    let mut fixture = Fixture::new();
    let (left, _right) = fixture.both_panels_in_archives().await;
    drop(left);
    *fixture.next.lock().unwrap() = Next::Fails;
    let failed = fixture.open(PaneId::Left, 2, "b.7z", Some(1)).await;
    assert!(
        failed
            .unwrap_err()
            .message
            .contains("unreadable test archive")
    );
    fixture.ended(1);
    assert_eq!(fixture.listed_sessions(), [2]);
    assert_eq!(fixture.allowance.counted(), 1);
    while fixture.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn a_panicking_open_returns_its_slot() {
    let mut fixture = Fixture::new();
    let (left, _right) = fixture.both_panels_in_archives().await;
    drop(left);
    *fixture.next.lock().unwrap() = Next::Panics;
    assert!(
        fixture
            .open(PaneId::Left, 2, "b.7z", Some(1))
            .await
            .is_err()
    );
    fixture.ended(1);
    assert_eq!(
        fixture.allowance.counted(),
        1,
        "the unwound open kept no slot"
    );
    while fixture.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn a_cancelled_successor_is_withdrawn() {
    let mut fixture = Fixture::new();
    let (left, _right) = fixture.both_panels_in_archives().await;
    // A viewer or job still holds the left archive, so it does not end.
    let holder = left.location.mount.clone();
    drop(left);
    fixture.start(PaneId::Left, 2, "b.7z", Some(1)).await;
    fixture.successor_waits().await;
    // Escape or another navigation of the panel cancels the waiting one.
    fixture.controller.cancel(PaneId::Left);
    let cancelled = fixture.listed(PaneId::Left, NavigationId(2)).await;
    assert_eq!(cancelled.unwrap_err().kind, std::io::ErrorKind::Interrupted);
    assert_eq!(fixture.allowance.successors(), 0);
    drop(holder);
    fixture.ended(1);
    assert_eq!(
        fixture.allowance.counted(),
        1,
        "the withdrawn successor did not take the slot"
    );
    while fixture.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn a_held_session_ends_the_wait_after_its_deadline() {
    let mut fixture = Fixture::new();
    let (left, _right) = fixture.both_panels_in_archives().await;
    // A viewer or job still holds the left archive, so it never ends.
    let holder = left.location.mount.clone();
    drop(left);
    fixture.start(PaneId::Left, 2, "b.7z", Some(1)).await;
    fixture.successor_waits().await;
    // Only the deadline can end the wait now; the paused clock reaches it at once.
    tokio::time::pause();
    tokio::time::advance(HANDOFF_DEADLINE).await;
    refused(fixture.listed(PaneId::Left, NavigationId(2)).await);
    assert_eq!(fixture.allowance.successors(), 0);
    assert_eq!(
        fixture.listed_sessions(),
        [1, 2],
        "the held session still counts"
    );
    drop(holder);
    fixture.ended(1);
    assert_eq!(
        fixture.allowance.counted(),
        1,
        "the expired successor was withdrawn"
    );
    while fixture.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn without_a_session_to_replace_a_full_allowance_refuses_at_once() {
    let mut fixture = Fixture::new();
    let (_left, _right) = fixture.both_panels_in_archives().await;
    let error = refused(fixture.open(PaneId::Left, 2, "b.7z", None).await);
    assert_eq!(fixture.allowance.successors(), 0, "nothing waited");
    assert_eq!(error.path, std::env::temp_dir().join("b.7z"));
    // A session that another tab, panel or viewer holds is not named: the same result.
    let other = fixture.open(PaneId::Right, 2, "c.7z", None).await;
    refused(other);
    assert_eq!(fixture.listed_sessions(), [1, 2]);
    while fixture.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn opening_one_archive_while_one_slot_is_free_needs_no_replacement() {
    let mut fixture = Fixture::new();
    let first = fixture.open(PaneId::Left, 1, "a.7z", None).await.unwrap();
    let second = fixture
        .open(PaneId::Right, 1, "x.7z", Some(99))
        .await
        .unwrap();
    assert_eq!(fixture.listed_sessions(), [1, 2]);
    drop((first, second));
    while fixture.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn a_refusal_names_the_open_sessions_and_the_open_to_retry_before_its_failure() {
    let mut fixture = Fixture::new();
    let (_left, _right) = fixture.both_panels_in_archives().await;
    fixture.start(PaneId::Left, 2, "b.7z", None).await;
    let (sessions, retry) = loop {
        let event = tokio::time::timeout(Duration::from_secs(60), fixture.events.recv())
            .await
            .expect("the refusal arrives")
            .expect("engine events");
        match event {
            Event::ArchiveSessionsFull {
                pane: PaneId::Left,
                navigation: NavigationId(2),
                sessions,
                retry,
            } => break (sessions, retry),
            Event::ArchiveListed { .. } => panic!("the failure came before the sessions"),
            _ => {}
        }
    };
    let named: Vec<_> = sessions
        .iter()
        .map(|session| (session.id, session.container.clone()))
        .collect();
    let temp = std::env::temp_dir();
    assert_eq!(named, [(1, temp.join("a.7z")), (2, temp.join("x.7z"))]);
    assert!(matches!(
        retry,
        ArchiveAction::List(location)
            if location.mount.is_none() && location.address.container == temp.join("b.7z")
    ));
    refused(fixture.listed(PaneId::Left, NavigationId(2)).await);
    while fixture.tasks.join_next().await.is_some() {}
}
