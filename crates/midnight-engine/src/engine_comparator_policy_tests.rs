//! Final current-policy teardown must use its shared cleanup reservation.
use crate::engine_command_admission_fixture::Case;
use midnight_core::{
    Command, ComparatorValidationProof, CompiledFileComparatorPolicy, Event,
    FileComparatorPreferences, FsError, PreparedFileComparatorPolicy, SettingsService,
    UserPreferences,
};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Duration,
};

struct DropRecord {
    thread: thread::ThreadId,
    name: Option<String>,
}

struct Compiled {
    raw: Arc<FileComparatorPreferences>,
    dropped: mpsc::Sender<DropRecord>,
}
impl CompiledFileComparatorPolicy for Compiled {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.raw
    }
    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        None
    }
    fn matcher_budget_bytes(&self) -> usize {
        1
    }
}
impl Drop for Compiled {
    fn drop(&mut self) {
        let current = thread::current();
        let _ = self.dropped.send(DropRecord {
            thread: current.id(),
            name: current.name().map(str::to_owned),
        });
    }
}

fn assert_retired(observed: mpsc::Receiver<DropRecord>, caller: thread::ThreadId) {
    let record = observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(record.thread, caller);
    assert_eq!(
        record.name.as_deref(),
        Some("fm-retirement"),
        "final current-policy Drop must use retirement, including after blocking work returns"
    );
}

async fn assert_retired_async(observed: mpsc::Receiver<DropRecord>) {
    let caller = thread::current().id();
    // Keep polling aborted actor futures while the independent cleanup worker
    // waits for their last shared reference. Blocking this executor would mask
    // the actual ownership boundary behind a test-only scheduler deadlock.
    tokio::time::timeout(
        Duration::from_secs(6),
        tokio::task::spawn_blocking(move || {
            assert_retired(observed, caller);
        }),
    )
    .await
    .unwrap()
    .unwrap();
}

fn probe(engine: &crate::Engine) -> mpsc::Receiver<DropRecord> {
    let store = engine.comparator_policies();
    let (dropped, observed) = mpsc::channel();
    store.publish(
        PreparedFileComparatorPolicy::new(Arc::new(Compiled {
            raw: Arc::default(),
            dropped,
        }))
        .with_id(store.prepare_id().unwrap()),
    );
    // Do not mask final internal-owner destruction with an external raw Arc.
    drop(store);
    observed
}

#[derive(Default)]
struct LoadGate {
    entered: tokio::sync::Notify,
    release: (Mutex<bool>, Condvar),
}
impl LoadGate {
    fn block(&self) {
        self.entered.notify_one();
        let released = self.release.0.lock().unwrap();
        let (released, timeout) = self
            .release
            .1
            .wait_timeout_while(released, Duration::from_secs(12), |released| !*released)
            .unwrap();
        assert!(
            *released && !timeout.timed_out(),
            "settings gate was never released"
        );
    }
    fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.entered.notified())
            .await
            .unwrap();
    }
}
struct ReleaseGate(Arc<LoadGate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}
#[derive(Default)]
struct Settings {
    gate: Option<Arc<LoadGate>>,
}
impl SettingsService for Settings {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
    }
    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        if let Some(gate) = &self.gate {
            gate.block();
        }
        Ok(UserPreferences::default())
    }
    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        assert!(proof.matches(&preferences.file_comparators));
        Ok(())
    }
}

#[test]
fn dropping_engine_before_run_retires_current_compiled_policy_on_worker() {
    let case = Case::new();
    let (engine, _events) = case.engine(8, false);
    let store = engine.comparator_policies();
    let (dropped, observed) = mpsc::channel();
    let id = store.prepare_id().unwrap();
    store.publish(
        PreparedFileComparatorPolicy::new(Arc::new(Compiled {
            raw: Arc::default(),
            dropped,
        }))
        .with_id(id),
    );
    drop(store);
    let caller = thread::current().id();
    drop(engine);
    assert_retired(observed, caller);
}

#[test]
fn dropping_unpolled_run_retires_current_policy_without_service_anchor_retention() {
    let case = Case::new();
    let (mut engine, _events) = case.engine(8, false);
    let store = engine.comparator_policies();
    let (dropped, observed) = mpsc::channel();
    store.publish(
        PreparedFileComparatorPolicy::new(Arc::new(Compiled {
            raw: Arc::default(),
            dropped,
        }))
        .with_id(store.prepare_id().unwrap()),
    );
    let (sender, receiver) = engine.command_channel(1).unwrap();
    let run = engine.run(receiver);
    drop(store);
    let caller = thread::current().id();
    drop(run);
    assert_retired(observed, caller);
    drop(sender);
}

#[tokio::test(flavor = "current_thread")]
async fn fully_polled_engine_without_settings_retires_current_policy_off_polling_thread() {
    let case = Case::new();
    let (mut engine, _events) = case.engine(128, false);
    let observed = probe(&engine);
    let (sender, receiver) = engine.command_channel(1).unwrap();
    sender.try_send(Command::Shutdown).unwrap();
    tokio::time::timeout(Duration::from_secs(5), engine.run(receiver))
        .await
        .unwrap();
    assert_retired_async(observed).await;
}

#[tokio::test(flavor = "current_thread")]
async fn configured_settings_normal_shutdown_retires_last_current_policy_off_polling_thread() {
    let case = Case::new();
    let (mut engine, mut events) = case.engine(128, false);
    engine
        .configure_settings()
        .unwrap()
        .install(Arc::new(Settings::default()));
    let observed = probe(&engine);
    let (sender, receiver) = engine.command_channel(2).unwrap();
    let run = tokio::spawn(engine.run(receiver));
    sender.try_send(Command::LoadPreferences).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Event::PreferencesLoaded(result) = events.recv().await.unwrap() {
                assert!(result.is_ok());
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        observed.try_recv().is_err(),
        "running preferences retains the current store"
    );
    sender.try_send(Command::Shutdown).unwrap();
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap();
    assert_retired_async(observed).await;
}

#[tokio::test(flavor = "current_thread")]
async fn aborted_engine_and_late_settings_return_keep_final_policy_retirement_guarded() {
    let case = Case::new();
    let (mut engine, _events) = case.engine(128, false);
    let gate = Arc::new(LoadGate::default());
    let release = ReleaseGate(gate.clone());
    engine
        .configure_settings()
        .unwrap()
        .install(Arc::new(Settings {
            gate: Some(gate.clone()),
        }));
    let observed = probe(&engine);
    let (sender, receiver) = engine.command_channel(2).unwrap();
    let run = tokio::spawn(engine.run(receiver));
    sender.try_send(Command::LoadPreferences).unwrap();
    gate.entered().await;
    run.abort();
    assert!(run.await.unwrap_err().is_cancelled());
    assert!(
        observed.try_recv().is_err(),
        "in-flight settings still owns the policy"
    );
    drop(release);
    assert_retired_async(observed).await;
}

#[test]
fn settings_return_after_shutdown_grace_and_runtime_exit_retires_on_cleanup_worker() {
    let case = Case::new();
    let (mut engine, _events) = case.engine(128, false);
    let gate = Arc::new(LoadGate::default());
    let release = ReleaseGate(gate.clone());
    engine
        .configure_settings()
        .unwrap()
        .install(Arc::new(Settings {
            gate: Some(gate.clone()),
        }));
    let observed = probe(&engine);
    let (sender, receiver) = engine.command_channel(2).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let run = tokio::spawn(engine.run(receiver));
        sender.try_send(Command::LoadPreferences).unwrap();
        gate.entered().await;
        sender.try_send(Command::Shutdown).unwrap();
        // The real two-second settings grace expires while this callback is blocked.
        tokio::time::timeout(Duration::from_secs(7), run)
            .await
            .unwrap()
            .unwrap();
        assert!(
            observed.try_recv().is_err(),
            "late blocking callback retains its guarded store"
        );
    });
    runtime.shutdown_timeout(Duration::from_millis(10));
    assert!(observed.try_recv().is_err());
    drop(release);
    assert_retired(observed, thread::current().id());
}
