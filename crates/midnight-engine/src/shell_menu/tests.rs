use super::*;
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

fn input() -> Input {
    Input {
        request: DesktopRequestId(1),
        files: SelectedFiles {
            selection: Arc::new(SelectionSnapshot::default()),
            view: Arc::new(DirectoryIndex::default()),
            fallback: Some(std::env::temp_dir().join("_build/menu-fixture")),
        },
        owner: 0,
        position: Default::default(),
        modifiers: Default::default(),
        target: ShellMenuTarget::Items,
    }
}

struct PanicMenu(Mutex<Option<oneshot::Sender<()>>>);
impl ShellMenuService for PanicMenu {
    fn show_shell_menu(&self, _: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        self.0.lock().unwrap().take().unwrap().send(()).unwrap();
        panic!("injected native extension panic");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_panic_keeps_menu_admission_bounded_until_failure_is_delivered() {
    let (sender, mut events) = mpsc::channel(1);
    sender
        .try_send(Event::LocaleLoaded(Ok(Default::default())))
        .unwrap();
    let (started, observed) = oneshot::channel();
    let controller = Controller::new(
        Some(Arc::new(PanicMenu(Mutex::new(Some(started))))),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    let mut tasks = JoinSet::new();
    controller.show(input(), &mut tasks).await;
    tokio::time::timeout(Duration::from_secs(2), observed)
        .await
        .unwrap()
        .unwrap();
    let prematurely_released = tokio::time::timeout(Duration::from_millis(100), async {
        while controller.budget.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    // Drain before asserting, so even a failed regression cannot leave a background send.
    events.recv().await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    tasks.join_next().await.unwrap().unwrap();
    assert!(
        !prematurely_released,
        "native panic released the menu budget while its failure reply was blocked"
    );
    assert!(matches!(
        event,
        Event::ShellMenuFinished { result: Err(_), .. }
    ));
    assert_eq!(controller.budget.available_permits(), 1);
}

#[derive(Default)]
struct Latch {
    released: Mutex<bool>,
    wake: Condvar,
}
struct ReleaseOnDrop(Arc<Latch>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        *self.0.released.lock().unwrap() = true;
        self.0.wake.notify_all();
    }
}
struct BlockingMenu {
    started: Mutex<Option<oneshot::Sender<()>>>,
    latch: Arc<Latch>,
}
impl ShellMenuService for BlockingMenu {
    fn show_shell_menu(&self, _: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        self.started
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        let mut released = self.latch.released.lock().unwrap();
        while !*released {
            released = self.latch.wake.wait(released).unwrap();
        }
        Ok(ShellMenuOutcome::Dismissed)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborting_reply_task_keeps_menu_budget_until_native_worker_returns() {
    let (sender, _events) = mpsc::channel(1);
    let (started, observed) = oneshot::channel();
    let latch = Arc::new(Latch::default());
    let release = ReleaseOnDrop(latch.clone());
    let controller = Controller::new(
        Some(Arc::new(BlockingMenu {
            started: Mutex::new(Some(started)),
            latch,
        })),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
    );
    let mut tasks = JoinSet::new();
    controller.show(input(), &mut tasks).await;
    tokio::time::timeout(Duration::from_secs(2), observed)
        .await
        .unwrap()
        .unwrap();
    tasks.abort_all();
    assert!(tasks.join_next().await.unwrap().is_err());
    assert_eq!(
        controller.budget.available_permits(),
        0,
        "aborting the reply cannot free a still-running native worker"
    );
    drop(release);
    tokio::time::timeout(Duration::from_secs(2), async {
        while controller.budget.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
