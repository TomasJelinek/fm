use super::*;
use midnight_core::{ChecksumAlgorithm, PathPickerKind, PathPickerOwner};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::sync::mpsc;

struct FakePicker {
    calls: AtomicUsize,
    wait: bool,
    bad: bool,
}
impl PathPickerService for FakePicker {
    fn pick_path(
        &self,
        _: &PathPickerRequest,
        cancel: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        while self.wait && !cancel.is_cancelled() {
            std::thread::park_timeout(Duration::from_millis(5));
        }
        Ok(self.bad.then(|| "relative path".into()))
    }
}

fn input() -> PathPickerRequest {
    PathPickerRequest {
        kind: PathPickerKind::File,
        title: "Choose file".into(),
        initial_directory: None,
        owner: PathPickerOwner::None,
    }
}

fn controller(service: Option<Arc<dyn PathPickerService>>) -> (Controller, mpsc::Receiver<Event>) {
    let (sender, receiver) = mpsc::channel(4);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    (
        Controller::new(TaskLifetime::testing(service.clone()), service, outbox),
        receiver,
    )
}

async fn event(
    receiver: &mut mpsc::Receiver<Event>,
) -> (DesktopRequestId, Result<Option<PathBuf>, FsError>) {
    let Event::PathPicked { request, result } =
        tokio::time::timeout(Duration::from_secs(3), receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("Unexpected event")
    };
    (request, result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_one_picker_is_admitted_and_cancellation_targets_its_request() {
    let service = Arc::new(FakePicker {
        calls: AtomicUsize::new(0),
        wait: true,
        bad: false,
    });
    let (mut picker, mut receiver) = controller(Some(service.clone()));
    let mut tasks = JoinSet::new();
    picker.show(DesktopRequestId(1), input(), &mut tasks).await;
    picker.cancel(DesktopRequestId(9));
    assert!(!picker.active.as_ref().unwrap().1.is_cancelled());
    picker.show(DesktopRequestId(2), input(), &mut tasks).await;
    let (id, result) = event(&mut receiver).await;
    assert_eq!(id, DesktopRequestId(2));
    assert!(result.is_err());
    assert_eq!(picker.budget.available_permits(), 0);
    picker.cancel(DesktopRequestId(1));
    let (id, result) = event(&mut receiver).await;
    assert_eq!(id, DesktopRequestId(1));
    assert!(result.unwrap().is_none());
    tokio::time::timeout(Duration::from_secs(3), tasks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(picker.budget.available_permits(), 1);
    assert!(service.calls.load(Ordering::SeqCst) <= 1);
}

#[tokio::test]
async fn unavailable_picker_and_malformed_result_have_explicit_errors() {
    for service in [
        None,
        Some(Arc::new(FakePicker {
            calls: AtomicUsize::new(0),
            wait: false,
            bad: true,
        }) as Arc<dyn PathPickerService>),
    ] {
        let (mut picker, mut receiver) = controller(service);
        let mut tasks = JoinSet::new();
        picker.show(DesktopRequestId(1), input(), &mut tasks).await;
        assert!(event(&mut receiver).await.1.is_err());
        tasks.join_next().await.unwrap().unwrap();
        assert_eq!(picker.budget.available_permits(), 1);
    }
}

#[tokio::test]
async fn invalid_suggestion_is_rejected_before_native_work() {
    let service = Arc::new(FakePicker {
        calls: AtomicUsize::new(0),
        wait: false,
        bad: false,
    });
    let (mut picker, mut receiver) = controller(Some(service.clone()));
    let mut tasks = JoinSet::new();
    let mut request = input();
    request.initial_directory = Some("relative".into());
    picker.show(DesktopRequestId(1), request, &mut tasks).await;
    assert!(event(&mut receiver).await.1.is_err());
    assert!(tasks.is_empty());
    assert_eq!(service.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn checksum_pickers_keep_kind_owner_and_raw_path_on_the_backend_worker() {
    struct CapturingPicker {
        caller: std::thread::ThreadId,
        expected: PathPickerKind,
        path: PathBuf,
    }
    impl PathPickerService for CapturingPicker {
        fn pick_path(
            &self,
            request: &PathPickerRequest,
            _: &Cancellation,
        ) -> Result<Option<PathBuf>, FsError> {
            assert_ne!(std::thread::current().id(), self.caller);
            assert_eq!(request.kind, self.expected);
            assert_eq!(request.owner, PathPickerOwner::Windows(123));
            assert_eq!(request.initial_directory.as_deref(), self.path.parent());
            Ok(Some(self.path.clone()))
        }
    }
    let root = std::env::current_dir().unwrap();
    let selected = root.join("Příliš 中文 e\u{301} 😀.sha256");
    for kind in [
        PathPickerKind::ChecksumOpen,
        PathPickerKind::ChecksumSave(ChecksumAlgorithm::Sha256),
    ] {
        let service = Arc::new(CapturingPicker {
            caller: std::thread::current().id(),
            expected: kind,
            path: selected.clone(),
        });
        let (mut picker, mut receiver) = controller(Some(service));
        let mut tasks = JoinSet::new();
        let mut request = input();
        request.kind = kind;
        request.owner = PathPickerOwner::Windows(123);
        request.initial_directory = Some(root.clone());
        picker.show(DesktopRequestId(17), request, &mut tasks).await;
        let (id, result) = event(&mut receiver).await;
        assert_eq!(id, DesktopRequestId(17));
        assert_eq!(result.unwrap(), Some(selected.clone()));
        tasks.join_next().await.unwrap().unwrap();
        assert_eq!(picker.budget.available_permits(), 1);
    }
}

#[path = "path_picker_lifetime_tests.rs"]
mod lifetime_tests;
