use super::*;
use midnight_core::FileComparatorProgram;
use midnight_plugin_host::{ContentSource, Error};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "desktop_external_compare_launch_tests.rs"]
mod launch_tests;

struct Session(Arc<AtomicUsize>);
impl ExternalCompareSessionCredit for Session {}
impl Drop for Session {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[derive(Clone, Copy, Debug)]
enum Cleanup {
    Success,
    Fail,
    Panic,
    Unknown,
}
#[derive(Debug)]
struct Source {
    cleanup: Cleanup,
    releases: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    callback_threads: Arc<std::sync::Mutex<Vec<std::thread::ThreadId>>>,
}
impl ContentSource for Source {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "metadata only".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        panic!("external preparation must not read content")
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        self.callback_threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        self.releases.fetch_add(1, Ordering::SeqCst);
        match self.cleanup {
            Cleanup::Success => Ok(()),
            Cleanup::Fail | Cleanup::Unknown => {
                std::mem::forget(self);
                Err(Error("unconfirmed test cleanup".into()))
            }
            Cleanup::Panic => {
                std::mem::forget(self);
                panic!("release callback panicked");
            }
        }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Fixture {
    front: ExternalComparisonFrontDoor,
    domain: RetirementDomain,
    releases: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    sessions: Arc<AtomicUsize>,
    callback_threads: Arc<std::sync::Mutex<Vec<std::thread::ThreadId>>>,
}
impl Fixture {
    fn new() -> Self {
        let domain = RetirementDomain::try_start(6, 0).unwrap();
        Self {
            front: ExternalComparisonFrontDoor::new(domain.clone()),
            domain,
            releases: Arc::default(),
            drops: Arc::default(),
            reads: Arc::default(),
            sessions: Arc::default(),
            callback_threads: Arc::default(),
        }
    }
    fn capture(&self, modes: [Cleanup; 2]) -> ExternalCompareNativeCapture {
        let calls = AtomicUsize::new(0);
        let (releases, drops, reads) = (
            self.releases.clone(),
            self.drops.clone(),
            self.reads.clone(),
        );
        let callback_threads = self.callback_threads.clone();
        Arc::new(move |_, _| {
            let cleanup = modes[calls.fetch_add(1, Ordering::SeqCst)];
            Ok(CapturedFile::from_source(Arc::new(Source {
                cleanup,
                releases: releases.clone(),
                drops: drops.clone(),
                reads: reads.clone(),
                callback_threads: callback_threads.clone(),
            }))
            .unwrap())
        })
    }
    fn prepare(
        &self,
        capture: &ExternalCompareNativeCapture,
        cancel: Cancellation,
    ) -> Result<ExternalCompareRequest, FsError> {
        let paths = paths();
        let (reply, _) = std::sync::mpsc::sync_channel(1);
        self.front.prepare(
            process(&paths),
            paths,
            capture,
            Arc::new(Session(self.sessions.clone())),
            cancel,
            reply,
        )
    }
}
fn paths() -> [PathBuf; 2] {
    let root = std::env::current_dir().unwrap();
    [root.join("left.txt"), root.join("right.txt")]
}
fn process(paths: &[PathBuf; 2]) -> ExternalCompareProcess {
    FileComparatorProgram {
        executable: std::env::current_dir().unwrap().join("compare.exe"),
        arguments: vec!["{left}".into(), "{right}".into()],
        working_directory: None,
    }
    .process_for(paths)
    .unwrap()
}

#[test]
fn second_capture_failure_releases_first_without_reading_content() {
    let fixture = Fixture::new();
    let real = fixture.capture([Cleanup::Success; 2]);
    let calls = AtomicUsize::new(0);
    let capture: ExternalCompareNativeCapture = Arc::new(move |path, cancel| {
        if calls.fetch_add(1, Ordering::SeqCst) == 1 {
            Err(ExternalCompareCaptureError {
                error: failure("B failed"),
                cleanup_unconfirmed: false,
            })
        } else {
            real(path, cancel)
        }
    });
    assert!(fixture.prepare(&capture, Cancellation::default()).is_err());
    assert_eq!(fixture.releases.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.reads.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.front.open_budget().available_permits(), 4);
    assert_eq!(fixture.sessions.load(Ordering::SeqCst), 1);
}

#[test]
fn cancelled_successful_capture_is_armed_before_cancellation_check() {
    let fixture = Fixture::new();
    let real = fixture.capture([Cleanup::Success; 2]);
    let capture: ExternalCompareNativeCapture = Arc::new(move |path, cancel| {
        let captured = real(path, cancel)?;
        cancel.cancel();
        Ok(captured)
    });
    assert!(fixture.prepare(&capture, Cancellation::default()).is_err());
    assert_eq!(fixture.releases.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn uncertain_capture_failure_retains_actual_slot_open_and_session_credit() {
    let fixture = Fixture::new();
    let capture: ExternalCompareNativeCapture = Arc::new(|_, _| {
        Err(ExternalCompareCaptureError {
            error: failure("native failure"),
            cleanup_unconfirmed: true,
        })
    });
    assert!(fixture.prepare(&capture, Cancellation::default()).is_err());
    assert_eq!(fixture.front.open_budget().available_permits(), 3);
    assert_eq!(fixture.sessions.load(Ordering::SeqCst), 0);
    let held: Vec<_> = (0..5)
        .map(|_| fixture.domain.try_reserve().unwrap())
        .collect();
    assert!(fixture.domain.try_reserve().is_err());
    drop(held);
}

#[test]
fn failed_or_panicking_release_on_either_side_retains_original_pair_accounting() {
    for side in 0..2 {
        for mode in [Cleanup::Fail, Cleanup::Panic, Cleanup::Unknown] {
            let fixture = Fixture::new();
            let mut modes = [Cleanup::Success; 2];
            modes[side] = mode;
            let request = fixture
                .prepare(&fixture.capture(modes), Cancellation::default())
                .unwrap();
            let armed = fixture
                .front
                .classify(request)
                .unwrap_or_else(|_| panic!("origin"));
            assert!(matches!(
                armed.work.finish_on_worker(|pair| finish_pair(pair, true)),
                CheckedFinish::Unconfirmed(_)
            ));
            assert_eq!(fixture.front.open_budget().available_permits(), 3);
            assert_eq!(fixture.sessions.load(Ordering::SeqCst), 0);
            assert_eq!(fixture.drops.load(Ordering::SeqCst), side);
            assert_eq!(fixture.reads.load(Ordering::SeqCst), 0);
            let held: Vec<_> = (0..5)
                .map(|_| fixture.domain.try_reserve().unwrap())
                .collect();
            assert!(fixture.domain.try_reserve().is_err());
            drop(held);
        }
    }
}

#[test]
fn mixed_four_open_one_properties_one_intake_ledger_refuses_seventh_before_capture() {
    let fixture = Fixture::new();
    let properties = fixture.domain.try_reserve().unwrap();
    let intake = fixture.domain.try_reserve().unwrap();
    let mut requests = Vec::new();
    for _ in 0..4 {
        requests.push(
            fixture
                .prepare(
                    &fixture.capture([Cleanup::Success; 2]),
                    Cancellation::default(),
                )
                .unwrap(),
        );
    }
    let never_capture: ExternalCompareNativeCapture =
        Arc::new(|_, _| panic!("capture before admission"));
    assert!(
        fixture
            .prepare(&never_capture, Cancellation::default())
            .is_err()
    );
    assert!(fixture.domain.try_reserve().is_err());
    drop((requests, properties, intake));
}

fn channel(fixture: &Fixture, capacity: usize) -> (crate::CommandSender, crate::CommandReceiver) {
    let copies = RetirementDomain::try_start(1, 0).unwrap();
    let inbox = RetirementDomain::try_start(1, 0)
        .unwrap()
        .try_reserve()
        .unwrap();
    let (sender, receiver) = crate::CommandSender::channel(
        capacity,
        crate::copy_operations::Admission::new(copies),
        inbox,
    );
    (sender.with_external(fixture.front.clone()), receiver)
}
async fn cleanup(fixture: &Fixture, count: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while fixture.drops.load(Ordering::SeqCst) != count
            || fixture.front.open_budget().available_permits() != 4
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn all_four_sender_routes_classify_comparators_into_guarded_messages() {
    use midnight_core::Command;
    for route in 0..4 {
        let fixture = Fixture::new();
        let (sender, mut receiver) = channel(&fixture, 1);
        let request = fixture
            .prepare(
                &fixture.capture([Cleanup::Success; 2]),
                Cancellation::default(),
            )
            .unwrap();
        let command = Command::LaunchExternalComparison(request);
        match route {
            0 => sender.try_send(command).unwrap(),
            1 => sender.send(command).await.unwrap(),
            2 => sender.reserve().await.unwrap().try_send(command).unwrap(),
            _ => sender
                .clone()
                .reserve_owned()
                .await
                .unwrap()
                .try_send(command)
                .unwrap(),
        }
        assert_eq!(
            fixture.releases.load(Ordering::SeqCst),
            0,
            "sender invokes no callback"
        );
        let message = receiver.inbox.recv().await.unwrap();
        assert!(matches!(
            &message,
            crate::engine_commands::Message::GuardedExternalComparison(_)
        ));
        drop(message);
        cleanup(&fixture, 2).await;
    }
}

#[tokio::test]
async fn unpolled_send_and_unpolled_refusal_result_already_own_checked_discard() {
    use midnight_core::Command;
    for refused_result in [false, true] {
        let fixture = Fixture::new();
        let (sender, _receiver) = channel(&fixture, 1);
        sender.try_send(Command::Shutdown).unwrap();
        let request = fixture
            .prepare(
                &fixture.capture([Cleanup::Success; 2]),
                Cancellation::default(),
            )
            .unwrap();
        if refused_result {
            let result = sender.try_send(Command::LaunchExternalComparison(request));
            assert!(result.is_err());
            // A caller may defer examining the synchronous refusal result.
            drop(async move { result });
        } else {
            drop(sender.send(Command::LaunchExternalComparison(request)));
        }
        cleanup(&fixture, 2).await;
        assert!(
            fixture
                .callback_threads
                .lock()
                .unwrap()
                .iter()
                .all(|thread| *thread != std::thread::current().id())
        );
        assert_eq!(fixture.front.open_budget().available_permits(), 4);
    }
}

#[tokio::test]
async fn uncertain_cleanup_survives_unpolled_future_and_queued_receiver_shutdown() {
    for route in 0..3 {
        for mode in [Cleanup::Fail, Cleanup::Panic, Cleanup::Unknown] {
            let fixture = Fixture::new();
            let (sender, receiver) = channel(&fixture, 1);
            let (reply, outcome) = std::sync::mpsc::sync_channel(1);
            let paths = paths();
            let request = fixture
                .front
                .prepare(
                    process(&paths),
                    paths,
                    &fixture.capture([mode, Cleanup::Success]),
                    Arc::new(Session(fixture.sessions.clone())),
                    Cancellation::default(),
                    reply,
                )
                .unwrap();
            let command = midnight_core::Command::LaunchExternalComparison(request);
            if route == 2 {
                sender.try_send(command).unwrap();
                drop(receiver);
            } else if route == 1 {
                sender.try_send(midnight_core::Command::Shutdown).unwrap();
                let result = sender.try_send(command);
                assert!(result.is_err());
                drop(async move { result });
                drop(receiver);
            } else {
                drop(sender.send(command));
                drop(receiver);
            }
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while fixture.releases.load(Ordering::Acquire) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(
                matches!(
                    outcome.recv_timeout(std::time::Duration::from_secs(3)),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                ),
                "quarantined captures must not retain the outcome sender"
            );
            assert_eq!(fixture.front.open_budget().available_permits(), 3);
            assert_eq!(fixture.sessions.load(Ordering::SeqCst), 0);
            assert_eq!(fixture.drops.load(Ordering::SeqCst), 0);
            assert!(
                fixture
                    .callback_threads
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|thread| *thread != std::thread::current().id())
            );
            let held: Vec<_> = (0..5)
                .map(|_| fixture.domain.try_reserve().unwrap())
                .collect();
            assert!(fixture.domain.try_reserve().is_err());
            drop(held);
        }
    }
}

#[test]
fn unknown_content_and_resource_implementations_retain_pair_instead_of_using_drop() {
    #[derive(Debug)]
    struct Unknown(Arc<AtomicUsize>);
    impl ContentSource for Unknown {
        fn length(&self) -> u64 {
            0
        }
        fn display_name(&self) -> String {
            "unknown".into()
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
            panic!("content read")
        }
        // Deliberately retain the real unknown default consuming release.
    }
    impl midnight_plugin_host::ContentResources for Unknown {
        fn capture(&self, _: &[String]) -> Result<CapturedFile, Error> {
            panic!("resource capture")
        }
    }
    impl Drop for Unknown {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    for unknown_resources in [false, true] {
        let fixture = Fixture::new();
        let dropped = Arc::new(AtomicUsize::new(0));
        let normal = fixture.capture([Cleanup::Success; 2]);
        let capture: ExternalCompareNativeCapture = {
            let dropped = dropped.clone();
            Arc::new(move |path, cancel| {
                let unknown = Arc::new(Unknown(dropped.clone()));
                Ok(if unknown_resources {
                    normal(path, cancel)?.with_resources(unknown)
                } else {
                    CapturedFile::from_source(unknown).unwrap()
                })
            })
        };
        let request = fixture.prepare(&capture, Cancellation::default()).unwrap();
        let armed = fixture
            .front
            .classify(request)
            .unwrap_or_else(|_| panic!("origin"));
        assert!(matches!(
            armed.work.finish_on_worker(|pair| finish_pair(pair, false)),
            CheckedFinish::Unconfirmed(_)
        ));
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.front.open_budget().available_permits(), 3);
        assert_eq!(fixture.sessions.load(Ordering::SeqCst), 0);
        let held: Vec<_> = (0..5)
            .map(|_| fixture.domain.try_reserve().unwrap())
            .collect();
        assert!(fixture.domain.try_reserve().is_err());
        drop(held);
    }
}

#[tokio::test]
async fn preclose_fifo_permit_returns_intact_guarded_request() {
    let fixture = Fixture::new();
    let (sender, mut receiver) = channel(&fixture, 1);
    let permit = sender.reserve_owned().await.unwrap();
    receiver.inbox.close();
    let request = fixture
        .prepare(
            &fixture.capture([Cleanup::Success; 2]),
            Cancellation::default(),
        )
        .unwrap();
    let refused = permit.try_send(midnight_core::Command::LaunchExternalComparison(request));
    assert!(matches!(
        refused,
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
    ));
    drop(refused);
    cleanup(&fixture, 2).await;
}

#[tokio::test]
async fn foreign_engine_request_is_refused_intact_and_receiver_shutdown_is_guarded() {
    let fixture = Fixture::new();
    let foreign = Fixture::new();
    let (sender, receiver) = channel(&fixture, 1);
    let request = foreign
        .prepare(
            &foreign.capture([Cleanup::Success; 2]),
            Cancellation::default(),
        )
        .unwrap();
    let refused = sender.try_send(midnight_core::Command::LaunchExternalComparison(request));
    assert!(refused.is_err());
    drop(refused);
    cleanup(&foreign, 2).await;
    let request = fixture
        .prepare(
            &fixture.capture([Cleanup::Success; 2]),
            Cancellation::default(),
        )
        .unwrap();
    sender
        .try_send(midnight_core::Command::LaunchExternalComparison(request))
        .unwrap();
    drop(receiver);
    cleanup(&fixture, 2).await;
}

#[test]
fn cancellation_before_or_during_final_revalidation_skips_pending_side_and_releases_both() {
    use std::sync::atomic::AtomicBool;
    #[derive(Debug)]
    struct CancelSource {
        cancel: Cancellation,
        trigger: Arc<AtomicBool>,
        validations: Arc<AtomicUsize>,
        releases: Arc<AtomicUsize>,
    }
    impl ContentSource for CancelSource {
        fn length(&self) -> u64 {
            0
        }
        fn display_name(&self) -> String {
            "metadata".into()
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> Result<(), Error> {
            self.validations.fetch_add(1, Ordering::SeqCst);
            if self.trigger.load(Ordering::Acquire) {
                self.cancel.cancel();
            }
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
            panic!("content read")
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
            self.releases.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    for cancelled_before in [false, true] {
        let fixture = Fixture::new();
        let cancel = Cancellation::default();
        let trigger = Arc::new(AtomicBool::new(false));
        let validations = Arc::new(AtomicUsize::new(0));
        let capture: ExternalCompareNativeCapture = {
            let (cancel, trigger, validations, releases) = (
                cancel.clone(),
                trigger.clone(),
                validations.clone(),
                fixture.releases.clone(),
            );
            Arc::new(move |_, _| {
                Ok(CapturedFile::from_source(Arc::new(CancelSource {
                    cancel: cancel.clone(),
                    trigger: trigger.clone(),
                    validations: validations.clone(),
                    releases: releases.clone(),
                }))
                .unwrap())
            })
        };
        let request = fixture.prepare(&capture, cancel.clone()).unwrap();
        validations.store(0, Ordering::Release);
        trigger.store(true, Ordering::Release);
        if cancelled_before {
            cancel.cancel();
        }
        let armed = fixture
            .front
            .classify(request)
            .unwrap_or_else(|_| panic!("origin"));
        assert!(matches!(
            armed.work.finish_on_worker(|pair| finish_pair(pair, true)),
            CheckedFinish::Confirmed { result: Err(_), .. }
        ));
        assert_eq!(
            validations.load(Ordering::SeqCst),
            usize::from(!cancelled_before)
        );
        assert_eq!(fixture.releases.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.front.open_budget().available_permits(), 4);
    }
}

#[tokio::test]
async fn foreign_dynamic_owner_cannot_redirect_type_inspection_to_an_armed_inner_owner() {
    struct Redirect {
        protected: Box<dyn ExternalCompareOwnedPayload>,
        dropped: Arc<AtomicUsize>,
    }
    impl ExternalCompareOwnedPayload for Redirect {
        fn as_any(&self) -> &(dyn Any + Send) {
            self.protected.as_any()
        }
        fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
            panic!("foreign conversion invoked")
        }
    }
    impl Drop for Redirect {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    let fixture = Fixture::new();
    let request = fixture
        .prepare(
            &fixture.capture([Cleanup::Success; 2]),
            Cancellation::default(),
        )
        .unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    let foreign = ExternalCompareRequest::from_backend_owner(Box::new(Redirect {
        protected: request.into_backend_owner(),
        dropped: dropped.clone(),
    }));
    let same = fixture
        .front
        .classify(foreign)
        .err()
        .expect("unknown dynamic owner refused");
    let owner = same.into_backend_owner();
    assert_eq!(owner.as_ref().type_id(), TypeId::of::<Redirect>());
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(owner);
    cleanup(&fixture, 2).await;
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}
