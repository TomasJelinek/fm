use super::*;
use midnight_core::{Event, FsError, PreferenceTheme, UiDensity};
use std::{
    io,
    path::Path,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::ThreadId,
    time::Duration,
};
use tokio::sync::{Notify, mpsc};

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: (Mutex<bool>, Condvar),
}

impl Gate {
    fn block(&self) {
        self.entered.notify_one();
        let mut released = self.release.0.lock().unwrap();
        while !*released {
            released = self.release.1.wait(released).unwrap();
        }
    }
    fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
    async fn wait(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.entered.notified())
            .await
            .expect("Settings service must execute on a backend worker");
    }
}

struct ReleaseOnDrop(Arc<Gate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Default)]
struct Settings {
    persisted: Mutex<UserPreferences>,
    writes: Mutex<Vec<UserPreferences>>,
    reads: AtomicUsize,
    calls: Mutex<Vec<ThreadId>>,
    load_gate: Option<Arc<Gate>>,
    save_gate: Option<Arc<Gate>>,
    fail_save: AtomicBool,
    panic_load: AtomicBool,
    panic_save: AtomicBool,
}

impl SettingsService for Settings {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        self.calls.lock().unwrap().push(std::thread::current().id());
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.panic_load.swap(false, Ordering::SeqCst) {
            panic!("Simulated settings panic");
        }
        if let Some(gate) = &self.load_gate {
            gate.block();
        }
        Ok(self.persisted.lock().unwrap().clone())
    }
    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        self.calls.lock().unwrap().push(std::thread::current().id());
        self.writes.lock().unwrap().push(preferences.clone());
        if self.panic_save.swap(false, Ordering::SeqCst) {
            panic!("Simulated save panic");
        }
        if let Some(gate) = &self.save_gate {
            gate.block();
        }
        if self.fail_save.swap(false, Ordering::SeqCst) {
            return Err(FsError::from_io(
                "save preferences",
                Path::new("settings"),
                io::ErrorKind::PermissionDenied.into(),
            ));
        }
        *self.persisted.lock().unwrap() = preferences;
        Ok(())
    }
}

fn preferences(theme: PreferenceTheme) -> UserPreferences {
    UserPreferences {
        theme,
        density: UiDensity::Touch,
        font_family: format!("文档 é 🧪 {theme:?}"),
        desktop_font_size: 12,
        touch_font_size: 20,
        ..UserPreferences::default()
    }
}

fn controller(service: Arc<Settings>) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        crate::task_lifetime::TaskLifetime::testing(service.clone()),
        service,
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
        .expect("Settings operation must acknowledge completion")
        .expect("Settings output closed")
}

async fn stop(controller: &Controller, tasks: &mut JoinSet<()>) {
    controller.shutdown();
    tokio::time::timeout(Duration::from_secs(3), controller.finished())
        .await
        .expect("Settings completion must follow its final native write");
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("Settings worker must stop without waiting for a full output queue");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_loads_coalesce_into_one_native_call_without_blocking_the_runtime() {
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let service = Arc::new(Settings {
        load_gate: Some(gate.clone()),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    assert!(controller.load());
    gate.wait().await;
    for _ in 0..1_000 {
        controller.load();
    }
    tokio::time::timeout(Duration::from_millis(100), tokio::task::yield_now())
        .await
        .unwrap();
    assert_eq!(service.reads.load(Ordering::SeqCst), 1);
    assert!(events.try_recv().is_err());
    gate.release();
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesLoaded(Ok(_))
    ));
    assert!(events.try_recv().is_err());
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_save_burst_retains_only_the_inflight_write_and_latest_pending_revision() {
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let service = Arc::new(Settings {
        save_gate: Some(gate.clone()),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    let first = preferences(PreferenceTheme::Dark);
    let latest = preferences(PreferenceTheme::Light);
    assert!(controller.save(PreferencesRevision(1), first.clone()));
    gate.wait().await;
    for revision in 2..=1_001 {
        assert!(controller.save(PreferencesRevision(revision), latest.clone()));
    }
    assert_eq!(*service.writes.lock().unwrap(), vec![first.clone()]);
    gate.release();
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(1),
            result: Ok(())
        }
    ));
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(1_001),
            result: Ok(())
        }
    ));
    assert_eq!(*service.writes.lock().unwrap(), vec![first, latest.clone()]);
    assert_eq!(*service.persisted.lock().unwrap(), latest);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_revision_is_idempotent_and_conflicting_or_stale_values_cannot_retarget_it() {
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let service = Arc::new(Settings {
        save_gate: Some(gate.clone()),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    let accepted = preferences(PreferenceTheme::Dark);
    let other = preferences(PreferenceTheme::Light);
    assert!(controller.save(PreferencesRevision(5), accepted.clone()));
    gate.wait().await;
    assert!(controller.save(PreferencesRevision(5), accepted.clone()));
    assert!(!controller.save(PreferencesRevision(5), other.clone()));
    assert!(!controller.save(PreferencesRevision(4), other.clone()));
    gate.release();
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(5),
            result: Ok(())
        }
    ));
    assert_eq!(*service.writes.lock().unwrap(), vec![accepted.clone()]);
    assert_eq!(*service.persisted.lock().unwrap(), accepted);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saving_is_serialized_after_a_slow_load_and_preserves_the_latest_request() {
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let service = Arc::new(Settings {
        load_gate: Some(gate.clone()),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    controller.load();
    gate.wait().await;
    controller.save(PreferencesRevision(1), preferences(PreferenceTheme::Dark));
    controller.save(PreferencesRevision(2), preferences(PreferenceTheme::Light));
    assert!(service.writes.lock().unwrap().is_empty());
    gate.release();
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesLoaded(Ok(_))
    ));
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(2),
            result: Ok(())
        }
    ));
    assert_eq!(
        *service.writes.lock().unwrap(),
        vec![preferences(PreferenceTheme::Light)]
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_save_is_reported_and_a_newer_revision_can_still_persist() {
    let service = Arc::new(Settings {
        fail_save: AtomicBool::new(true),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    controller.save(PreferencesRevision(1), preferences(PreferenceTheme::Light));
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(1),
            result: Err(FsError {
                kind: io::ErrorKind::PermissionDenied,
                ..
            })
        }
    ));
    assert_eq!(
        *service.persisted.lock().unwrap(),
        UserPreferences::default()
    );
    let accepted = preferences(PreferenceTheme::Dark);
    controller.save(PreferencesRevision(2), accepted.clone());
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(2),
            result: Ok(())
        }
    ));
    assert_eq!(*service.persisted.lock().unwrap(), accepted);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_panics_become_error_events_and_do_not_kill_the_settings_worker() {
    let service = Arc::new(Settings {
        panic_load: AtomicBool::new(true),
        panic_save: AtomicBool::new(true),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    controller.load();
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesLoaded(Err(FsError {
            kind: io::ErrorKind::Other,
            ..
        }))
    ));
    controller.save(PreferencesRevision(1), preferences(PreferenceTheme::Light));
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(1),
            result: Err(FsError {
                kind: io::ErrorKind::Other,
                ..
            })
        }
    ));
    controller.save(PreferencesRevision(2), preferences(PreferenceTheme::Dark));
    assert!(matches!(
        next(&mut events).await,
        Event::PreferencesSaved {
            revision: PreferencesRevision(2),
            result: Ok(())
        }
    ));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_does_not_wait_for_a_full_output_queue_or_accept_more_requests() {
    let service = Arc::new(Settings::default());
    let (sender, mut events) = mpsc::channel(1);
    sender
        .send(Event::PreferencesLoaded(Ok(Box::default())))
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        crate::task_lifetime::TaskLifetime::testing(service.clone()),
        service.clone(),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller.load();
    tokio::time::timeout(Duration::from_secs(3), async {
        while service.reads.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    stop(&controller, &mut tasks).await;
    assert!(!controller.load());
    assert!(!controller.save(PreferencesRevision(1), preferences(PreferenceTheme::Dark)));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::PreferencesLoaded(Ok(_)))
    ));
    assert!(events.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quick_change_and_close_drains_the_latest_save_after_the_inflight_write() {
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let service = Arc::new(Settings {
        save_gate: Some(gate.clone()),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    let first = preferences(PreferenceTheme::Dark);
    let last = preferences(PreferenceTheme::Light);
    controller.save(PreferencesRevision(1), first.clone());
    gate.wait().await;
    controller.save(PreferencesRevision(2), last.clone());
    controller.shutdown();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), controller.finished())
            .await
            .is_err()
    );
    assert!(events.try_recv().is_err());
    gate.release();
    stop(&controller, &mut tasks).await;
    assert_eq!(*service.writes.lock().unwrap(), vec![first, last.clone()]);
    assert_eq!(*service.persisted.lock().unwrap(), last);
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn shutdown_before_worker_runs_skips_load_and_persists_the_latest_pending_save() {
    let service = Arc::new(Settings::default());
    let (controller, mut tasks, mut events) = controller(service.clone());
    controller.load();
    controller.save(PreferencesRevision(1), preferences(PreferenceTheme::Dark));
    let last = preferences(PreferenceTheme::Light);
    controller.save(PreferencesRevision(2), last.clone());
    controller.shutdown();
    stop(&controller, &mut tasks).await;
    assert_eq!(service.reads.load(Ordering::SeqCst), 0);
    assert_eq!(*service.writes.lock().unwrap(), vec![last.clone()]);
    assert_eq!(*service.persisted.lock().unwrap(), last);
    assert!(events.try_recv().is_err());
}
