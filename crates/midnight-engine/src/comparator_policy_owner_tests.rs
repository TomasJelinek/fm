//! External policy owners survive dormant UI, queued closures and runtime loss.
use super::*;
use crate::retirement::RetirementDomain;
use midnight_core::{CompiledFileComparatorPolicy, PreparedFileComparatorPolicy};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

struct Credit(Arc<AtomicUsize>);
impl ExternalCompareSessionCredit for Credit {}
impl Drop for Credit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
/// A session credit that only counts the credits alive: sessions are not limited.
fn admit(active: &Arc<AtomicUsize>) -> Arc<dyn ExternalCompareSessionCredit> {
    active.fetch_add(1, Ordering::AcqRel);
    Arc::new(Credit(active.clone()))
}

struct Gate {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Drop for Gate {
    fn drop(&mut self) {
        let _ = self.entered.send(());
        let _ = self.release.recv_timeout(Duration::from_secs(10));
    }
}
struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
fn gate() -> (Gate, mpsc::Receiver<()>, Release) {
    let (entered, observed) = mpsc::channel();
    let (release, sender) = mpsc::channel();
    (
        Gate {
            entered,
            release: sender,
        },
        observed,
        Release(Some(release)),
    )
}

struct Compiled {
    raw: Arc<FileComparatorPreferences>,
    dropped: mpsc::Sender<(thread::ThreadId, Option<String>)>,
    gate: std::sync::Mutex<Option<Gate>>,
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
        drop(self.gate.get_mut().unwrap().take());
        let thread = thread::current();
        let _ = self
            .dropped
            .send((thread.id(), thread.name().map(str::to_owned)));
    }
}
fn publish(
    source: &ComparisonPolicySource,
    gate: Option<Gate>,
) -> mpsc::Receiver<(thread::ThreadId, Option<String>)> {
    let (dropped, observed) = mpsc::channel();
    source.policies.publish(
        PreparedFileComparatorPolicy::new(Arc::new(Compiled {
            raw: Arc::default(),
            dropped,
            gate: std::sync::Mutex::new(gate),
        }))
        .with_id(source.policies.prepare_id().unwrap()),
    );
    observed
}
/// A source whose retirement domain has room for its store's slot and nothing else:
/// pins reserve no slot.
fn source() -> ComparisonPolicySource {
    let retirement = RetirementDomain::try_start(1, 0).unwrap();
    ComparisonPolicySource::new(ComparatorPolicies::new(
        retirement.try_reserve().unwrap(),
        Arc::new(FileComparatorPolicyStore::default()),
    ))
}
fn assert_retired(observed: mpsc::Receiver<(thread::ThreadId, Option<String>)>) {
    let (id, name) = observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(id, thread::current().id());
    assert_eq!(name.as_deref(), Some("fm-retirement"));
}

#[test]
fn dormant_source_after_engine_and_runtime_exit_retires_current_policy() {
    let case = crate::engine_command_admission_fixture::Case::new();
    let (engine, _events) = case.engine(8, false);
    let source = engine.comparator_policy_source();
    let observed = publish(&source, None);
    let dormant = move || source.snapshot();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    drop(engine);
    drop(runtime);
    assert!(observed.try_recv().is_err());
    drop(dormant);
    assert_retired(observed);
}

/// Sessions are not counted: any number pin the current set without a cleanup slot of
/// their own, and pins of one generation share its set.
#[test]
fn any_number_of_pins_share_one_set_and_reserve_no_cleanup_slot() {
    let source = source();
    let active = Arc::new(AtomicUsize::new(0));
    let observed = publish(&source, None);
    let pins: Vec<_> = (0..6)
        .map(|_| source.pin(&source.snapshot(), admit(&active)).unwrap())
        .collect();
    assert_eq!(active.load(Ordering::Acquire), 6);
    assert!(
        pins.iter()
            .all(|pin| Arc::ptr_eq(pin.preferences(), pins[0].preferences()))
    );
    drop(pins);
    assert_eq!(
        active.load(Ordering::Acquire),
        0,
        "pins return their credits"
    );
    assert!(
        observed.try_recv().is_err(),
        "the store keeps its current set"
    );
    drop(source);
    assert_retired(observed);
}

/// Dropping the last pin of a displaced set never destroys it; the next publication
/// does, on the settings writer.
#[test]
fn a_historical_set_is_destroyed_by_the_next_publication_after_its_last_pin() {
    let source = source();
    let active = Arc::new(AtomicUsize::new(0));
    let first = publish(&source, None);
    let a = source.pin(&source.snapshot(), admit(&active)).unwrap();
    let b = source.pin(&source.snapshot(), admit(&active)).unwrap();
    let first_raw = a.preferences().clone();
    let second = publish(&source, None);
    let c = source.pin(&source.snapshot(), admit(&active)).unwrap();
    assert!(Arc::ptr_eq(&first_raw, b.preferences()));
    assert!(!Arc::ptr_eq(a.preferences(), c.preferences()));
    assert_eq!(source.policies.pinned_history(), 1);
    drop((a, b));
    assert!(first.try_recv().is_err(), "a pin never destroys a set");
    let writer = source.clone();
    let third = thread::Builder::new()
        .name("settings-writer".into())
        .spawn(move || publish(&writer, None))
        .unwrap()
        .join()
        .unwrap();
    let (_, name) = first.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(name.as_deref(), Some("settings-writer"));
    // The second set, still pinned, waits in the history.
    assert_eq!(source.policies.pinned_history(), 1);
    drop(c);
    assert!(second.try_recv().is_err());
    drop(source);
    assert_retired(second);
    assert_retired(third);
}

#[test]
fn accepted_pin_in_aborted_uninvoked_blocking_closure_retires_after_runtime_exit() {
    let source = source();
    let active = Arc::new(AtomicUsize::new(0));
    let old = publish(&source, None);
    let pin = source.pin(&source.snapshot(), admit(&active)).unwrap();
    let current = publish(&source, None);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (blocker, entered, release) = gate();
    runtime.spawn_blocking(move || drop(blocker));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let invoked = Arc::new(AtomicBool::new(false));
    let running = invoked.clone();
    let task = runtime.spawn_blocking(move || {
        running.store(true, Ordering::Release);
        drop(pin);
    });
    task.abort();
    drop(source);
    runtime.shutdown_timeout(Duration::ZERO);
    drop(release);
    // The pin was the store's last owner: the store and its history retire together.
    assert_retired(old);
    assert_retired(current);
    assert!(!invoked.load(Ordering::Acquire));
}

#[test]
fn unwind_after_acceptance_keeps_historical_policy_retirement_armed() {
    let source = source();
    let active = Arc::new(AtomicUsize::new(0));
    let old = publish(&source, None);
    let pin = source.pin(&source.snapshot(), admit(&active)).unwrap();
    let current = publish(&source, None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _pin = pin;
        panic!("injected comparison handoff unwind");
    }));
    assert!(result.is_err());
    assert!(
        old.try_recv().is_err(),
        "the unwound pin did not destroy it"
    );
    drop(source);
    assert_retired(old);
    assert_retired(current);
}

#[test]
fn stale_or_reconstructed_snapshot_is_refused_and_returns_its_credit_at_once() {
    let source = source();
    let active = Arc::new(AtomicUsize::new(0));
    let _old = publish(&source, None);
    let snapshot = source.snapshot();
    let first = source.pin(&snapshot, admit(&active)).unwrap();
    let _current = publish(&source, None);
    assert!(matches!(
        source.pin(&snapshot, admit(&active)),
        Err(ComparisonPolicyPinError::PolicyChanged)
    ));
    let mut wrong = source.snapshot();
    wrong.preferences = Arc::new((*wrong.preferences).clone());
    assert!(matches!(
        source.pin(&wrong, admit(&active)),
        Err(ComparisonPolicyPinError::PolicyChanged)
    ));
    assert_eq!(active.load(Ordering::Acquire), 1);
    let second = source.pin(&source.snapshot(), admit(&active)).unwrap();
    assert_eq!(active.load(Ordering::Acquire), 2);
    drop((first, second));
    assert_eq!(active.load(Ordering::Acquire), 0);
}
