//! External policy owners survive dormant UI, queued closures and runtime loss.
use super::*;
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
fn admit(active: &Arc<AtomicUsize>) -> Option<Arc<dyn ExternalCompareSessionCredit>> {
    active
        .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            (value < 2).then_some(value + 1)
        })
        .ok()?;
    Some(Arc::new(Credit(active.clone())))
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
fn source(capacity: usize) -> ComparisonPolicySource {
    let retirement = RetirementDomain::try_start(capacity, 0).unwrap();
    ComparisonPolicySource::new(
        ComparatorPolicies::new(
            retirement.try_reserve().unwrap(),
            Arc::new(FileComparatorPolicyStore::default()),
        ),
        retirement,
    )
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

#[test]
fn two_historical_pins_retire_independently_after_store_replacement() {
    let source = source(3);
    let active = Arc::new(AtomicUsize::new(0));
    let first = publish(&source, None);
    let a = source
        .pin(&source.snapshot(), admit(&active).unwrap())
        .unwrap();
    let first_raw = a.preferences().clone();
    let second = publish(&source, None);
    let b = source
        .pin(&source.snapshot(), admit(&active).unwrap())
        .unwrap();
    let current = publish(&source, None);
    assert!(Arc::ptr_eq(&first_raw, a.preferences()));
    assert!(!Arc::ptr_eq(a.preferences(), b.preferences()));
    assert!(admit(&active).is_none());
    let uninvoked = move || drop(a);
    drop(uninvoked);
    assert_retired(first);
    assert!(second.try_recv().is_err());
    assert!(current.try_recv().is_err());
    drop(b);
    assert_retired(second);
    drop(source);
    assert_retired(current);
}

#[test]
fn accepted_pin_in_aborted_uninvoked_blocking_closure_retires_after_runtime_exit() {
    let source = source(3);
    let active = Arc::new(AtomicUsize::new(0));
    let old = publish(&source, None);
    let pin = source
        .pin(&source.snapshot(), admit(&active).unwrap())
        .unwrap();
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
    assert_retired(old);
    assert_retired(current);
    assert!(!invoked.load(Ordering::Acquire));
}

#[test]
fn unwind_after_acceptance_keeps_historical_policy_retirement_armed() {
    let source = source(3);
    let active = Arc::new(AtomicUsize::new(0));
    let old = publish(&source, None);
    let pin = source
        .pin(&source.snapshot(), admit(&active).unwrap())
        .unwrap();
    let current = publish(&source, None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _pin = pin;
        panic!("injected comparison handoff unwind");
    }));
    assert!(result.is_err());
    assert_retired(old);
    drop(source);
    assert_retired(current);
}

#[test]
fn blocked_policy_cleanup_keeps_same_credit_until_actual_destructor_returns() {
    let source = source(4); // one extra test-only FIFO barrier slot
    let active = Arc::new(AtomicUsize::new(0));
    let (blocker, entered, release) = gate();
    let old = publish(&source, Some(blocker));
    let a = source
        .pin(&source.snapshot(), admit(&active).unwrap())
        .unwrap();
    let second = publish(&source, None);
    let b = source
        .pin(&source.snapshot(), admit(&active).unwrap())
        .unwrap();
    drop(a);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(active.load(Ordering::Acquire), 2);
    assert!(admit(&active).is_none());
    drop(release);
    assert_retired(old);
    // FIFO barrier observes payload credit destruction after the compiled Drop signal.
    let (done, observed) = mpsc::channel();
    struct Done(mpsc::Sender<()>);
    impl Drop for Done {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let lease = source.retirement.try_reserve().unwrap();
    drop(lease.own(Done(done)));
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    let next = admit(&active).expect("cleanup returned the original credit");
    drop(next);
    drop(b);
    drop(source);
    assert_retired(second);
}

#[test]
fn stale_second_open_returns_empty_slot_and_credit_before_immediate_retry() {
    let source = source(4); // store + two session slots + test-only blocking payload
    let active = Arc::new(AtomicUsize::new(0));
    let old = publish(&source, None);
    let snapshot = source.snapshot();
    let first = source.pin(&snapshot, admit(&active).unwrap()).unwrap();
    let _current = publish(&source, None);
    let (blocker, entered, release) = gate();
    drop(source.retirement.try_reserve().unwrap().own(blocker));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        source.pin(&snapshot, admit(&active).unwrap()),
        Err(ComparisonPolicyPinError::PolicyChanged)
    ));
    assert_eq!(active.load(Ordering::Acquire), 1);
    let refreshed = source.snapshot();
    let second = source.pin(&refreshed, admit(&active).unwrap()).unwrap();
    assert_eq!(active.load(Ordering::Acquire), 2);
    assert!(
        source.retirement.try_reserve().is_err(),
        "only four actual owners fit"
    );
    drop(release);
    drop(first);
    assert_retired(old);
    drop(second);
}

#[test]
fn same_id_different_arc_and_busy_release_unused_credit_synchronously() {
    let source = source(2);
    let active = Arc::new(AtomicUsize::new(0));
    let mut wrong = source.snapshot();
    wrong.preferences = Arc::new((*wrong.preferences).clone());
    assert!(matches!(
        source.pin(&wrong, admit(&active).unwrap()),
        Err(ComparisonPolicyPinError::PolicyChanged)
    ));
    assert_eq!(active.load(Ordering::Acquire), 0);
    let held = source.retirement.try_reserve().unwrap();
    assert!(matches!(
        source.pin(&source.snapshot(), admit(&active).unwrap()),
        Err(ComparisonPolicyPinError::RetirementBusy)
    ));
    assert_eq!(active.load(Ordering::Acquire), 0);
    drop(held);
    assert!(
        source
            .pin(&source.snapshot(), admit(&active).unwrap())
            .is_ok()
    );
}
