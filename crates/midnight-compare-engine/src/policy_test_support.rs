//! Provider-free fakes exercise the real Service/worker seam. They contain no
//! compiled resources and do NOT simulate or qualify Engine retirement behavior.
#![allow(dead_code)] // Shared knobs are consumed by independently owned route tests.
use crate::{FileComparatorPinError, FileComparatorPolicySource, OwnedFileComparatorPolicy};
use midnight_core::{
    ExternalCompareSessionCredit, FileComparatorPolicyStore, FileComparatorPreferences,
    FileComparatorRule, FileComparatorSnapshot, FileComparatorTarget, FilterMode,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::ThreadId,
    time::Duration,
};

pub(super) struct SelectionGate {
    pub(super) entered: SyncSender<()>,
    pub(super) release: Receiver<()>,
}

/// Raw metadata and observable test work only, never a compiled-policy owner.
pub(super) struct PolicyProbe {
    pub(super) raw: Arc<FileComparatorPreferences>,
    pub(super) seen: Mutex<Vec<[PathBuf; 2]>>,
    pub(super) selection_threads: Mutex<Vec<ThreadId>>,
    pub(super) selection_gate: Mutex<Option<SelectionGate>>,
    selected: Option<usize>,
}

impl PolicyProbe {
    pub(super) fn gate_next_selection(&self) -> (Receiver<()>, SyncSender<()>) {
        let (entered, observed) = mpsc::sync_channel(1);
        let (release, blocked) = mpsc::sync_channel(1);
        *self.selection_gate.lock().unwrap() = Some(SelectionGate {
            entered,
            release: blocked,
        });
        (observed, release)
    }
}

struct Published {
    snapshot: FileComparatorSnapshot,
    probe: Arc<PolicyProbe>,
}

pub(super) struct FakePolicySource {
    // TEST ONLY: the existing lightweight default store mints valid opaque IDs.
    // No fake compiler, real historical policy, publication or retirement occurs.
    ids: FileComparatorPolicyStore,
    current: Mutex<Published>,
    pin_error: Mutex<Option<FileComparatorPinError>>,
    pub(super) pins: AtomicUsize,
    pub(super) snapshot_reads: AtomicUsize,
    pub(super) live_pins: Arc<AtomicUsize>,
    pub(super) dropped_pins: Arc<AtomicUsize>,
    pub(super) credits: Mutex<Vec<Weak<dyn ExternalCompareSessionCredit>>>,
}

impl FakePolicySource {
    pub(super) fn new() -> Arc<Self> {
        let snapshot = FileComparatorSnapshot::default();
        let probe = Arc::new(PolicyProbe {
            raw: snapshot.preferences.clone(),
            seen: Mutex::default(),
            selection_threads: Mutex::default(),
            selection_gate: Mutex::new(None),
            selected: None,
        });
        Arc::new(Self {
            ids: FileComparatorPolicyStore::default(),
            current: Mutex::new(Published { snapshot, probe }),
            pin_error: Mutex::new(None),
            pins: AtomicUsize::new(0),
            snapshot_reads: AtomicUsize::new(0),
            live_pins: Arc::new(AtomicUsize::new(0)),
            dropped_pins: Arc::new(AtomicUsize::new(0)),
            credits: Mutex::default(),
        })
    }

    pub(super) fn publish(
        &self,
        target: FileComparatorTarget,
    ) -> (FileComparatorSnapshot, Arc<PolicyProbe>) {
        self.publish_rules(
            vec![FileComparatorRule {
                enabled: true,
                mode: FilterMode::Masks,
                expression: "*".into(),
                case_sensitive: false,
                target,
            }],
            Some(0),
        )
    }

    pub(super) fn publish_rules(
        &self,
        rules: Vec<FileComparatorRule>,
        selected: Option<usize>,
    ) -> (FileComparatorSnapshot, Arc<PolicyProbe>) {
        let snapshot = FileComparatorSnapshot {
            id: self.ids.prepare_id().unwrap(),
            preferences: Arc::new(FileComparatorPreferences { rules }),
        };
        let probe = Arc::new(PolicyProbe {
            raw: snapshot.preferences.clone(),
            seen: Mutex::default(),
            selection_threads: Mutex::default(),
            selection_gate: Mutex::new(None),
            selected,
        });
        *self.current.lock().unwrap() = Published {
            snapshot: snapshot.clone(),
            probe: probe.clone(),
        };
        (snapshot, probe)
    }

    pub(super) fn set_pin_error(&self, error: Option<FileComparatorPinError>) {
        *self.pin_error.lock().unwrap() = error;
    }
}

impl FileComparatorPolicySource for FakePolicySource {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.snapshot_reads.fetch_add(1, Ordering::AcqRel);
        self.current.lock().unwrap().snapshot.clone()
    }

    fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<Box<dyn OwnedFileComparatorPolicy>, FileComparatorPinError> {
        self.pins.fetch_add(1, Ordering::AcqRel);
        self.credits.lock().unwrap().push(Arc::downgrade(&session));
        if let Some(error) = *self.pin_error.lock().unwrap() {
            return Err(error);
        }
        let published = self.current.lock().unwrap();
        if !crate::policy::same_snapshot(expected, &published.snapshot) {
            return Err(FileComparatorPinError::PolicyChanged);
        }
        self.live_pins.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(FakeOwnedPolicy {
            snapshot: published.snapshot.clone(),
            probe: published.probe.clone(),
            live: self.live_pins.clone(),
            dropped: self.dropped_pins.clone(),
            _session: session,
        }))
    }
}

/// No compiled matcher is stored here. Dropping this fake records Service
/// ownership only; Backend's real-source gates own fm-retirement qualification.
struct FakeOwnedPolicy {
    snapshot: FileComparatorSnapshot,
    probe: Arc<PolicyProbe>,
    live: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    // The real Engine guarded payload also drops policy before this SAME credit.
    _session: Arc<dyn ExternalCompareSessionCredit>,
}

impl OwnedFileComparatorPolicy for FakeOwnedPolicy {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.snapshot.clone()
    }

    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.snapshot.preferences
    }

    fn select(&self, paths: &[PathBuf; 2]) -> Option<usize> {
        self.probe.seen.lock().unwrap().push(paths.clone());
        self.probe
            .selection_threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        let gate = self.probe.selection_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.try_send(()).unwrap();
            // Disconnect on assertion unwind also releases this test-only gate.
            let _ = gate.release.recv_timeout(Duration::from_secs(5));
        }
        if paths
            .iter()
            .any(|path| path.file_name().and_then(|name| name.to_str()).is_none())
        {
            return None;
        }
        self.probe.selected.filter(|index| {
            self.snapshot
                .preferences
                .rules
                .get(*index)
                .is_some_and(|rule| rule.enabled)
        })
    }
}

impl Drop for FakeOwnedPolicy {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::AcqRel);
        self.dropped.fetch_add(1, Ordering::AcqRel);
    }
}
