use super::*;
use midnight_core::UserPreferences;
use std::sync::{
    Condvar,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Default)]
struct Settings {
    calls: AtomicUsize,
    entered: Notify,
    release: (Mutex<bool>, Condvar),
    blocked: AtomicBool,
    fail: AtomicBool,
}
impl SettingsService for Settings {
    fn font_families(&self) -> Result<Vec<String>, FsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.blocked.load(Ordering::SeqCst) {
            let mut released = self.release.0.lock().unwrap();
            while !*released {
                released = self.release.1.wait(released).unwrap();
            }
        }
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(FsError::from_io(
                "enumerate fonts",
                Path::new(""),
                io::ErrorKind::PermissionDenied.into(),
            ));
        }
        Ok(vec!["文档 é 🧪".into(), "Segoe UI".into()])
    }
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
    }
}
struct Release(Arc<Settings>);
impl Drop for Release {
    fn drop(&mut self) {
        self.release();
    }
}
impl Release {
    fn release(&self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
fn setup(
    service: Arc<Settings>,
    capacity: usize,
) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(capacity);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        crate::task_lifetime::TaskLifetime::testing(service.clone()),
        Some(service),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    (controller, tasks, events)
}
async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn entered(service: &Settings) {
    tokio::time::timeout(Duration::from_secs(3), service.entered.notified())
        .await
        .unwrap();
}
async fn stop(controller: &Controller, tasks: &mut JoinSet<()>) {
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enumeration_coalesces_pending_requests_and_reuses_the_completed_catalog() {
    let service = Arc::new(Settings {
        blocked: AtomicBool::new(true),
        ..Settings::default()
    });
    let release = Release(service.clone());
    let (controller, mut tasks, mut events) = setup(service.clone(), 4);
    assert!(controller.list());
    entered(&service).await;
    for _ in 0..1000 {
        assert!(controller.list());
    }
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    assert!(events.try_recv().is_err());
    release.release();
    let Event::FontFamiliesLoaded(Ok(first)) = next(&mut events).await else {
        panic!("expected font catalog")
    };
    assert_eq!(first, ["文档 é 🧪", "Segoe UI"]);
    assert!(events.try_recv().is_err());
    assert!(controller.list());
    let Event::FontFamiliesLoaded(Ok(cached)) = next(&mut events).await else {
        panic!("expected cached catalog")
    };
    assert_eq!(cached, first);
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    stop(&controller, &mut tasks).await;
    assert!(!controller.list());
}

#[tokio::test]
async fn failed_enumeration_is_reported_and_can_be_retried() {
    let service = Arc::new(Settings {
        fail: AtomicBool::new(true),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = setup(service.clone(), 4);
    controller.list();
    assert!(matches!(
        next(&mut events).await,
        Event::FontFamiliesLoaded(Err(FsError {
            kind: io::ErrorKind::PermissionDenied,
            ..
        }))
    ));
    controller.list();
    assert!(matches!(
        next(&mut events).await,
        Event::FontFamiliesLoaded(Ok(_))
    ));
    assert_eq!(service.calls.load(Ordering::SeqCst), 2);
    stop(&controller, &mut tasks).await;
}

#[tokio::test]
async fn shutdown_does_not_wait_for_a_full_output_queue() {
    let service = Arc::new(Settings::default());
    let (controller, mut tasks, events) = setup(service.clone(), 1);
    controller.list();
    entered(&service).await;
    // First delivery fills the channel; a later cached request must not hold exit.
    tokio::time::timeout(Duration::from_secs(3), async {
        while events.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    controller.list();
    stop(&controller, &mut tasks).await;
    assert_eq!(events.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_blocked_font_scan_does_not_delay_preferences_saves() {
    let service = Arc::new(Settings {
        blocked: AtomicBool::new(true),
        ..Settings::default()
    });
    let release = Release(service.clone());
    let (fonts, mut tasks, mut events) = setup(service.clone(), 4);
    let (sender, mut saved) = mpsc::channel(4);
    let preferences = crate::preferences::Controller::new(
        crate::task_lifetime::TaskLifetime::testing(service.clone()),
        service.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    fonts.list();
    entered(&service).await;
    assert!(preferences.save(
        midnight_core::PreferencesRevision(1),
        UserPreferences {
            font_family: "字🧪".into(),
            ..UserPreferences::default()
        }
    ));
    assert!(matches!(
        next(&mut saved).await,
        Event::PreferencesSaved { result: Ok(()), .. }
    ));
    assert!(events.try_recv().is_err());
    preferences.shutdown();
    release.release();
    assert!(matches!(
        next(&mut events).await,
        Event::FontFamiliesLoaded(Ok(_))
    ));
    stop(&fonts, &mut tasks).await;
}
