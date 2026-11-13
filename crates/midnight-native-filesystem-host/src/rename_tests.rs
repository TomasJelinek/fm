//! Pure transport fixtures: no filesystem fixture, package load or native operation.
use crate::{Cancel, NativeProvider, Transport, test_path};
use midnight_core::{
    Cancellation, EntryKind, PreparedRename, RenameAttempt, RenameConflictToken, RenameFinish,
    RenameOutcome, RenamePolicy, RenamePrepareFailure, RenameService,
};
use midnight_filesystem_plugin_api as api;
use std::{
    collections::VecDeque,
    ffi::OsString,
    num::NonZeroU64,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, mpsc},
    time::{Duration, Instant},
};

const MOUNT: u64 = 11;
const NODE: u64 = 23;
const HANDLE: u64 = 37;
const SOURCE: &str = "selected žluťoučký.txt";

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn source_path() -> PathBuf {
    std::env::temp_dir()
        .join("host-prepared-rename-tests")
        .join(SOURCE)
}
fn text(value: &str) -> api::NativeText {
    api::NativeText {
        encoding: test_path::encoding(),
        bytes: test_path::name(value),
    }
}
fn metadata(kind: api::Kind, size: u64) -> api::Metadata {
    api::Metadata {
        size: (kind == api::Kind::File).then_some(size),
        modified: Some(api::Timestamp {
            seconds: 9,
            nanos: 7,
        }),
        compression: api::Compression::Compressed,
        ..api::Metadata::default()
    }
}
fn node(kind: api::Kind) -> api::Node {
    api::Node {
        id: NODE,
        address: test_path::address(&source_path()),
        name_encoding: test_path::encoding(),
        raw_name: test_path::name(SOURCE),
        display: "A deliberately unrelated display caption".into(),
        kind,
        metadata: metadata(kind, 3),
    }
}
fn prepared(kind: api::Kind) -> api::PreparedRename {
    api::PreparedRename {
        handle: HANDLE,
        source: api::RenameObject {
            identity: vec![0xaa, 0xbb],
            revision: vec![0xcc],
            name: text(SOURCE),
            kind,
            metadata: metadata(kind, 17),
        },
        parent_identity: vec![0xdd],
        recoverable_replace: kind == api::Kind::File,
    }
}
fn input(number: u64, name: &str, policy: RenamePolicy) -> RenameAttempt {
    RenameAttempt {
        number: NonZeroU64::new(number).unwrap(),
        name: name.into(),
        policy,
    }
}
fn rename_request(number: u64, name: &str, policy: api::RenamePolicy) -> api::Request {
    api::Request::Rename {
        prepared: HANDLE,
        attempt: number,
        name: text(name),
        policy,
    }
}
fn state(number: u64, outcome: api::RenameOutcome) -> api::Response {
    api::Response::RenameState(api::RenameState {
        prepared: HANDLE,
        attempt: number,
        outcome,
    })
}
fn renamed(number: u64, name: &str) -> api::Response {
    state(
        number,
        api::RenameOutcome::Renamed {
            name: text(name),
            same_entry: false,
            recovery: vec![],
            durability_confirmed: false,
        },
    )
}
fn conflict(number: u64, raw_token: u64, observed_name: &str) -> api::Response {
    state(
        number,
        api::RenameOutcome::Conflict(api::RenameConflict {
            token: raw_token,
            target: api::RenameObject {
                identity: vec![0x71],
                revision: vec![0x72],
                name: text(observed_name),
                kind: api::Kind::File,
                metadata: metadata(api::Kind::File, 29),
            },
            can_replace: true,
        }),
    )
}

#[derive(Debug, Default)]
struct GateState {
    entered: bool,
    open: bool,
}
#[derive(Debug, Default)]
struct CallGate {
    state: Mutex<GateState>,
    changed: Condvar,
}
impl CallGate {
    fn block(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut state = lock(&self.state);
        state.entered = true;
        self.changed.notify_all();
        while !state.open {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }
    fn wait_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = lock(&self.state);
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "The actual fixture callback did not enter"
            );
            state = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
    fn release(&self) {
        lock(&self.state).open = true;
        self.changed.notify_all();
    }
}
struct ReleaseGate(Arc<CallGate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[derive(Debug)]
enum Answer {
    Reply(Box<Result<api::Response, api::Failure>>),
    Panic,
}
#[derive(Debug)]
struct Step {
    request: api::Request,
    answer: Answer,
    cancel: Option<Cancellation>,
    gate: Option<Arc<CallGate>>,
}
impl Step {
    fn ok(request: api::Request, response: api::Response) -> Self {
        Self {
            request,
            answer: Answer::Reply(Box::new(Ok(response))),
            cancel: None,
            gate: None,
        }
    }
    fn error(request: api::Request, code: api::ErrorCode) -> Self {
        Self {
            request,
            answer: Answer::Reply(Box::new(Err(api::Failure::new(
                code,
                "Synthetic lost native reply",
            )))),
            cancel: None,
            gate: None,
        }
    }
    fn cancel(mut self, cancellation: &Cancellation) -> Self {
        self.cancel = Some(cancellation.clone());
        self
    }
    fn held(mut self, gate: &Arc<CallGate>) -> Self {
        self.gate = Some(gate.clone());
        self
    }
}
#[derive(Debug)]
struct Fake {
    steps: Mutex<VecDeque<Step>>,
    calls: Mutex<Vec<(api::Request, bool)>>,
    violations: Mutex<Vec<String>>,
}
impl Transport for Fake {
    fn call(
        &self,
        request: &api::Request,
        cancelled: Cancel<'_>,
    ) -> Result<api::Response, api::Failure> {
        lock(&self.calls).push((request.clone(), cancelled()));
        if !matches!(
            request,
            api::Request::Resolve { .. }
                | api::Request::PrepareRename { .. }
                | api::Request::Rename { .. }
                | api::Request::GetRenameState { .. }
                | api::Request::CloseRename { .. }
                | api::Request::CloseMount { .. }
        ) {
            lock(&self.violations)
                .push(format!("Unexpected content or other request: {request:?}"));
            return Err(api::Failure::new(
                api::ErrorCode::Invalid,
                "Forbidden fixture request",
            ));
        }
        let step = lock(&self.steps).pop_front();
        let Some(step) = step else {
            lock(&self.violations).push(format!("Unscripted request: {request:?}"));
            return Err(api::Failure::new(
                api::ErrorCode::Invalid,
                "Unscripted request",
            ));
        };
        if step.request != *request {
            lock(&self.violations).push(format!("Expected {:?}, got {request:?}", step.request));
            return Err(api::Failure::new(
                api::ErrorCode::Invalid,
                "Wrong fixture request",
            ));
        }
        if step.gate.as_ref().is_some_and(|gate| !gate.block()) {
            lock(&self.violations).push("Held actual callback timed out".into());
            return Err(api::Failure::new(
                api::ErrorCode::Io,
                "Fixture callback timed out",
            ));
        }
        if let Some(cancel) = step.cancel {
            cancel.cancel();
        }
        match step.answer {
            Answer::Reply(reply) => *reply,
            Answer::Panic => panic!("Synthetic provider panic after dispatch"),
        }
    }
}
struct Fixture {
    provider: NativeProvider,
    fake: Arc<Fake>,
}
impl Fixture {
    fn new(steps: Vec<Step>) -> Self {
        let fake = Arc::new(Fake {
            steps: Mutex::new(steps.into()),
            calls: Mutex::default(),
            violations: Mutex::default(),
        });
        Self {
            provider: NativeProvider {
                transport: fake.clone(),
                capabilities: api::abi::CAP_NATIVE_ADDRESS | api::abi::CAP_PREPARED_RENAME,
            },
            fake,
        }
    }
    fn prepare(&self) -> Arc<dyn PreparedRename> {
        match RenameService::prepare(&self.provider, &source_path(), &Cancellation::default()) {
            Ok(resource) => resource,
            Err(failure) => panic!("Expected prepared resource: {failure:?}"),
        }
    }
    fn check(&self) {
        let violations = lock(&self.fake.violations);
        assert!(violations.is_empty(), "{violations:?}");
        let steps = lock(&self.fake.steps);
        assert!(steps.is_empty(), "Unconsumed steps: {steps:?}");
    }
    fn calls(&self) -> Vec<(api::Request, bool)> {
        lock(&self.fake.calls).clone()
    }
}
fn preparation_failure(fixture: &Fixture, cancel: &Cancellation) -> RenamePrepareFailure {
    match RenameService::prepare(&fixture.provider, &source_path(), cancel) {
        Err(failure) => failure,
        Ok(resource) => {
            let _ = resource.finish();
            panic!("Invalid preparation unexpectedly exposed a usable preview")
        }
    }
}
fn opening(listed: api::Kind, pinned: api::Kind) -> Vec<Step> {
    vec![
        Step::ok(
            api::Request::Resolve {
                address: test_path::address(&source_path()),
            },
            api::Response::Resolved {
                mount: MOUNT,
                node: node(listed),
            },
        ),
        Step::ok(
            api::Request::PrepareRename {
                mount: MOUNT,
                node: NODE,
            },
            api::Response::RenamePrepared(prepared(pinned)),
        ),
    ]
}
fn closing() -> Vec<Step> {
    vec![
        Step::ok(
            api::Request::CloseRename { prepared: HANDLE },
            api::Response::Unit,
        ),
        Step::ok(
            api::Request::CloseMount { mount: MOUNT },
            api::Response::Unit,
        ),
    ]
}
fn assert_renamed(value: &midnight_core::RenameState, attempt: u64, expected_name: &str) {
    assert_eq!(value.attempt, attempt);
    match &value.outcome {
        RenameOutcome::Renamed {
            name, same_entry, ..
        } => {
            assert_eq!(name, &OsString::from(expected_name));
            assert!(!same_entry);
        }
        other => panic!("Expected Renamed, got {other:?}"),
    }
}

#[test]
fn prepared_rename_precancel_and_missing_capability_do_not_dispatch() {
    let cancel = Cancellation::default();
    cancel.cancel();
    let fixture = Fixture::new(vec![]);
    let failure = preparation_failure(&fixture, &cancel);
    assert_eq!(failure.error.kind, std::io::ErrorKind::Interrupted);
    assert!(failure.retained.is_none());
    assert!(fixture.calls().is_empty());
    fixture.check();
    for capabilities in [api::abi::CAP_NATIVE_ADDRESS, api::abi::CAP_PREPARED_RENAME] {
        let mut fixture = Fixture::new(vec![]);
        fixture.provider.capabilities = capabilities;
        let failure = preparation_failure(&fixture, &Cancellation::default());
        assert_eq!(failure.error.kind, std::io::ErrorKind::Unsupported);
        assert!(failure.retained.is_none());
        assert!(fixture.calls().is_empty());
        fixture.check();
    }
}

#[test]
fn prepared_rename_adopts_mount_before_bad_resolve_or_cancellation() {
    for mode in 0..3 {
        let cancellation = Cancellation::default();
        let mut resolved = node(api::Kind::File);
        if mode == 0 {
            resolved.address = test_path::address(
                &source_path()
                    .parent()
                    .unwrap()
                    .join("different-parent")
                    .join(SOURCE),
            );
        } else if mode == 1 {
            resolved.id = 0;
        }
        let mut resolve = Step::ok(
            api::Request::Resolve {
                address: test_path::address(&source_path()),
            },
            api::Response::Resolved {
                mount: MOUNT,
                node: resolved,
            },
        );
        if mode == 2 {
            resolve = resolve.cancel(&cancellation);
        }
        let fixture = Fixture::new(vec![
            resolve,
            Step::ok(
                api::Request::CloseMount { mount: MOUNT },
                api::Response::Unit,
            ),
        ]);
        let failure = preparation_failure(&fixture, &cancellation);
        assert!(failure.retained.is_none());
        if mode == 2 {
            assert_eq!(failure.error.kind, std::io::ErrorKind::Interrupted);
        }
        assert!(
            !fixture
                .calls()
                .iter()
                .any(|(request, _)| matches!(request, api::Request::PrepareRename { .. }))
        );
        fixture.check();
    }
}

#[test]
fn prepared_rename_adopts_handle_before_cancel_or_invalid_preview() {
    for mode in 0..4 {
        let cancellation = Cancellation::default();
        let mut pin = prepared(api::Kind::File);
        if mode == 0 {
            pin.source.name = text("different.txt");
        }
        if mode == 1 {
            pin.source.kind = api::Kind::Directory;
            pin.source.metadata.size = None;
            pin.recoverable_replace = false;
        }
        if mode == 2 {
            pin.source.metadata.modified.as_mut().unwrap().nanos = 1_000_000_000;
        }
        let mut steps = opening(api::Kind::File, api::Kind::File);
        steps[1] = Step::ok(
            api::Request::PrepareRename {
                mount: MOUNT,
                node: NODE,
            },
            api::Response::RenamePrepared(pin),
        );
        if mode == 3 {
            steps[1].cancel = Some(cancellation.clone());
        }
        steps.extend(closing());
        let fixture = Fixture::new(steps);
        let failure = preparation_failure(&fixture, &cancellation);
        assert!(failure.retained.is_none(), "Cleanup was fully acknowledged");
        if mode == 3 {
            assert_eq!(failure.error.kind, std::io::ErrorKind::Interrupted);
        }
        let calls = fixture.calls();
        assert!(matches!(
            &calls[calls.len() - 2..],
            [
                (api::Request::CloseRename { prepared: HANDLE }, false),
                (api::Request::CloseMount { mount: MOUNT }, false)
            ]
        ));
        fixture.check();
    }
}

#[test]
fn prepared_rename_preview_uses_pinned_metadata_and_link_hint_equivalence() {
    for (listed, pinned, expected) in [
        (api::Kind::File, api::Kind::File, EntryKind::File),
        (
            api::Kind::Directory,
            api::Kind::Directory,
            EntryKind::Directory,
        ),
        (
            api::Kind::DirectoryLink,
            api::Kind::Symlink,
            EntryKind::Symlink,
        ),
        (
            api::Kind::Symlink,
            api::Kind::DirectoryLink,
            EntryKind::DirectoryLink,
        ),
    ] {
        let mut steps = opening(listed, pinned);
        steps.extend(closing());
        let fixture = Fixture::new(steps);
        let resource = fixture.prepare();
        assert_eq!(resource.item().path, source_path());
        assert_eq!(resource.item().kind, expected);
        assert_eq!(
            resource.item().metadata.size,
            if pinned == api::Kind::File {
                Some(17)
            } else {
                None
            }
        );
        assert_eq!(
            resource.item().metadata.compression,
            midnight_core::CompressionStatus::Compressed
        );
        assert!(matches!(resource.finish(), RenameFinish::Closed));
        assert!(matches!(resource.finish(), RenameFinish::Closed));
        drop(resource);
        fixture.check();
    }
}

#[test]
fn prepared_rename_unknown_publication_retains_owner_without_blind_close() {
    for mode in 0..5 {
        let mut steps = if mode < 2 {
            vec![]
        } else {
            let mut steps = opening(api::Kind::File, api::Kind::File);
            steps.pop();
            steps
        };
        let request = if mode < 2 {
            api::Request::Resolve {
                address: test_path::address(&source_path()),
            }
        } else {
            api::Request::PrepareRename {
                mount: MOUNT,
                node: NODE,
            }
        };
        steps.push(match mode {
            0 | 2 => Step::error(request, api::ErrorCode::Io),
            1 | 3 => Step::ok(request, api::Response::Unit),
            _ => {
                let mut pin = prepared(api::Kind::File);
                pin.handle = 0;
                Step::ok(request, api::Response::RenamePrepared(pin))
            }
        });
        let fixture = Fixture::new(steps);
        let failure = preparation_failure(&fixture, &Cancellation::default());
        let retained = failure
            .retained
            .expect("Uncertain publication needs a disposal owner");
        assert!(matches!(retained.finish(), RenameFinish::Unconfirmed(_)));
        assert!(matches!(retained.finish(), RenameFinish::Unconfirmed(_)));
        drop(retained);
        assert!(!fixture.calls().iter().any(|(request, _)| matches!(
            request,
            api::Request::CloseMount { .. } | api::Request::CloseRename { .. }
        )));
        fixture.check();
    }
}

#[test]
fn prepared_rename_failed_validation_cleanup_returns_disposal_only_owner() {
    let mut steps = opening(api::Kind::File, api::Kind::File);
    let mut pin = prepared(api::Kind::File);
    pin.source.name = text("substitute.txt");
    steps[1] = Step::ok(
        api::Request::PrepareRename {
            mount: MOUNT,
            node: NODE,
        },
        api::Response::RenamePrepared(pin),
    );
    steps.push(Step::error(
        api::Request::CloseRename { prepared: HANDLE },
        api::ErrorCode::Io,
    ));
    let fixture = Fixture::new(steps);
    let failure = preparation_failure(&fixture, &Cancellation::default());
    let retained = failure
        .retained
        .expect("Unacknowledged pin disposal must stay owned");
    assert!(matches!(retained.finish(), RenameFinish::Unconfirmed(_)));
    drop(retained);
    assert!(
        !fixture
            .calls()
            .iter()
            .any(|(request, _)| matches!(request, api::Request::CloseMount { .. }))
    );
    fixture.check();
}

fn consent(value: &midnight_core::RenameState) -> RenameConflictToken {
    match &value.outcome {
        RenameOutcome::Conflict(conflict) => conflict.token,
        other => panic!("Expected conflict consent, got {other:?}"),
    }
}
fn mutation_count(fixture: &Fixture) -> usize {
    fixture
        .calls()
        .iter()
        .filter(|(request, _)| matches!(request, api::Request::Rename { .. }))
        .count()
}

#[test]
fn prepared_rename_consent_is_owner_bound_and_marshals_exact_provider_token() {
    let observed = if test_path::encoding() == api::NameEncoding::WindowsUtf16Le {
        "TARGET.txt"
    } else {
        "target.txt"
    };
    let mut first_steps = opening(api::Kind::File, api::Kind::File);
    first_steps.extend([
        Step::ok(
            rename_request(1, "target.txt", api::RenamePolicy::NoReplace),
            conflict(1, 42, observed),
        ),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            conflict(1, 42, observed),
        ),
        Step::ok(
            rename_request(
                2,
                "target.txt",
                api::RenamePolicy::RecoverableCurrentEntry { conflict: 42 },
            ),
            conflict(2, 43, observed),
        ),
        Step::ok(
            rename_request(3, "alternate.txt", api::RenamePolicy::NoReplace),
            conflict(3, 44, "alternate.txt"),
        ),
        Step::ok(
            rename_request(
                4,
                "alternate.txt",
                api::RenamePolicy::RecoverableCurrentEntry { conflict: 44 },
            ),
            renamed(4, "alternate.txt"),
        ),
    ]);
    first_steps.extend(closing());
    let first_fixture = Fixture::new(first_steps);
    let first = first_fixture.prepare();
    let _ = first.attempt(
        &input(
            1,
            "target.txt",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: RenameConflictToken(NonZeroU64::new(u64::MAX).unwrap()),
            },
        ),
        &Cancellation::default(),
    );
    assert_eq!(
        mutation_count(&first_fixture),
        0,
        "The first attempt requires NoReplace"
    );
    let requested = input(1, "target.txt", RenamePolicy::NoReplace);
    let first_conflict = first.attempt(&requested, &Cancellation::default());
    let first_token = consent(&first_conflict);
    if let RenameOutcome::Conflict(conflict) = &first_conflict.outcome {
        assert_eq!(
            conflict.target.path.file_name(),
            Some(std::ffi::OsStr::new(observed))
        );
        assert_eq!(conflict.target.metadata.size, Some(29));
        assert!(conflict.can_replace);
    }
    assert_eq!(consent(&first.query()), first_token);
    assert_eq!(
        consent(&first.attempt(&requested, &Cancellation::default())),
        first_token
    );
    let _ = first.attempt(
        &input(1, "changed.txt", RenamePolicy::NoReplace),
        &Cancellation::default(),
    );
    let _ = first.attempt(
        &input(
            2,
            "changed.txt",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first_token,
            },
        ),
        &Cancellation::default(),
    );
    assert_eq!(mutation_count(&first_fixture), 1);

    // Equal raw provider values from independent owners are different host consent.
    let mut second_steps = opening(api::Kind::File, api::Kind::File);
    second_steps.extend([
        Step::ok(
            rename_request(1, "target.txt", api::RenamePolicy::NoReplace),
            conflict(1, 42, observed),
        ),
        Step::ok(
            rename_request(
                2,
                "target.txt",
                api::RenamePolicy::RecoverableCurrentEntry { conflict: 42 },
            ),
            renamed(2, "target.txt"),
        ),
    ]);
    second_steps.extend(closing());
    let second_fixture = Fixture::new(second_steps);
    let second = second_fixture.prepare();
    let second_token = consent(&second.attempt(&requested, &Cancellation::default()));
    assert_ne!(first_token, second_token);
    let _ = second.attempt(
        &input(
            2,
            "target.txt",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first_token,
            },
        ),
        &Cancellation::default(),
    );
    assert_eq!(
        mutation_count(&second_fixture),
        1,
        "Foreign owner consent reached operation21"
    );
    assert_renamed(
        &second.attempt(
            &input(
                2,
                "target.txt",
                RenamePolicy::RecoverableCurrentEntry {
                    conflict: second_token,
                },
            ),
            &Cancellation::default(),
        ),
        2,
        "target.txt",
    );
    assert!(matches!(second.finish(), RenameFinish::Closed));
    drop(second);
    second_fixture.check();

    let changed_target = first.attempt(
        &input(
            2,
            "target.txt",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first_token,
            },
        ),
        &Cancellation::default(),
    );
    let current = consent(&changed_target);
    assert_ne!(current, first_token);
    let _ = first.attempt(
        &input(
            3,
            "target.txt",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first_token,
            },
        ),
        &Cancellation::default(),
    );
    let _ = first.attempt(
        &input(
            3,
            "alternate.txt",
            RenamePolicy::RecoverableCurrentEntry { conflict: current },
        ),
        &Cancellation::default(),
    );
    assert_eq!(mutation_count(&first_fixture), 2);
    let alternate = first.attempt(
        &input(3, "alternate.txt", RenamePolicy::NoReplace),
        &Cancellation::default(),
    );
    assert_renamed(
        &first.attempt(
            &input(
                4,
                "alternate.txt",
                RenamePolicy::RecoverableCurrentEntry {
                    conflict: consent(&alternate),
                },
            ),
            &Cancellation::default(),
        ),
        4,
        "alternate.txt",
    );
    assert!(matches!(first.finish(), RenameFinish::Closed));
    drop(first);
    first_fixture.check();
}

#[test]
fn prepared_rename_terminal_replay_and_late_cancel_never_repeat_effects() {
    for lost_reply in [false, true] {
        let cancellation = Cancellation::default();
        let mut steps = opening(api::Kind::File, api::Kind::File);
        let request = rename_request(1, "after.txt", api::RenamePolicy::NoReplace);
        steps.push(if lost_reply {
            Step::error(request, api::ErrorCode::Io).cancel(&cancellation)
        } else {
            Step::ok(request, renamed(1, "after.txt")).cancel(&cancellation)
        });
        if lost_reply {
            steps.push(Step::ok(
                api::Request::GetRenameState { prepared: HANDLE },
                renamed(1, "after.txt"),
            ));
        }
        steps.extend(closing());
        let fixture = Fixture::new(steps);
        let resource = fixture.prepare();
        // A skipped sequence must not dispatch or consume the valid first input.
        let _ = resource.attempt(
            &input(u64::MAX, "after.txt", RenamePolicy::NoReplace),
            &Cancellation::default(),
        );
        assert_eq!(mutation_count(&fixture), 0);
        let requested = input(1, "after.txt", RenamePolicy::NoReplace);
        assert_renamed(&resource.attempt(&requested, &cancellation), 1, "after.txt");
        assert!(cancellation.is_cancelled());
        assert_renamed(&resource.attempt(&requested, &cancellation), 1, "after.txt");
        let _ = resource.attempt(
            &input(1, "substitute.txt", RenamePolicy::NoReplace),
            &cancellation,
        );
        let _ = resource.attempt(
            &input(2, "later.txt", RenamePolicy::NoReplace),
            &cancellation,
        );
        assert_renamed(&resource.query(), 1, "after.txt");
        assert_eq!(mutation_count(&fixture), 1);
        let query_calls: Vec<_> = fixture
            .calls()
            .into_iter()
            .filter(|(request, _)| matches!(request, api::Request::GetRenameState { .. }))
            .collect();
        assert_eq!(query_calls.len(), usize::from(lost_reply));
        assert!(query_calls.iter().all(|(_, cancelled)| !cancelled));
        assert!(matches!(resource.finish(), RenameFinish::Closed));
        drop(resource);
        fixture.check();
    }
}

#[test]
fn prepared_rename_executing_queries_do_not_resend_mutation() {
    let mut steps = opening(api::Kind::File, api::Kind::File);
    steps.extend([
        Step::error(
            rename_request(1, "after.txt", api::RenamePolicy::NoReplace),
            api::ErrorCode::Io,
        ),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            state(1, api::RenameOutcome::Executing),
        ),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            state(1, api::RenameOutcome::Executing),
        ),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            renamed(1, "after.txt"),
        ),
    ]);
    steps.extend(closing());
    let fixture = Fixture::new(steps);
    let resource = fixture.prepare();
    let requested = input(1, "after.txt", RenamePolicy::NoReplace);
    assert!(matches!(
        resource
            .attempt(&requested, &Cancellation::default())
            .outcome,
        RenameOutcome::Executing
    ));
    assert!(matches!(
        resource
            .attempt(&requested, &Cancellation::default())
            .outcome,
        RenameOutcome::Executing
    ));
    assert!(matches!(resource.query().outcome, RenameOutcome::Executing));
    assert_renamed(&resource.query(), 1, "after.txt");
    assert_eq!(mutation_count(&fixture), 1);
    assert!(
        fixture
            .calls()
            .iter()
            .filter(|(request, _)| matches!(request, api::Request::GetRenameState { .. }))
            .all(|(_, cancelled)| !cancelled)
    );
    assert!(matches!(resource.finish(), RenameFinish::Closed));
    drop(resource);
    fixture.check();
}

#[test]
fn prepared_rename_untrusted_query_preserves_valid_partial_diagnostics() {
    // Before the first input, a refusal cannot invent a requested name or attempt.
    let mut before_input_steps = opening(api::Kind::File, api::Kind::File);
    before_input_steps.push(Step::ok(
        api::Request::GetRenameState { prepared: HANDLE },
        api::Response::Failed(api::Failure::new(
            api::ErrorCode::PermissionDenied,
            "Query refused before any mutation input",
        )),
    ));
    before_input_steps.extend(closing());
    let before_input_fixture = Fixture::new(before_input_steps);
    let before_input = before_input_fixture.prepare();
    let value = before_input.query();
    assert_eq!(value.attempt, 0);
    match &value.outcome {
        RenameOutcome::Unconfirmed {
            requested_name,
            last_known_attempt,
            error,
            ..
        } => {
            assert!(requested_name.is_empty());
            assert_eq!(*last_known_attempt, 0);
            assert_eq!(error.path, source_path());
        }
        other => panic!("Failed initial query fabricated state: {other:?}"),
    }
    assert!(matches!(before_input.finish(), RenameFinish::Closed));
    assert!(matches!(
        before_input.query().outcome,
        RenameOutcome::Unconfirmed { .. }
    ));
    assert_eq!(mutation_count(&before_input_fixture), 0);
    drop(before_input);
    before_input_fixture.check();

    let retained_path = source_path().parent().unwrap().join("retained.source");
    for mode in 0..5 {
        let mut reply = renamed(1, "after.txt");
        if let api::Response::RenameState(value) = &mut reply {
            match mode {
                0 => value.prepared = HANDLE + 1,
                1 => value.attempt = 2,
                2 => {
                    if let api::RenameOutcome::Renamed { name, .. } = &mut value.outcome {
                        *name = text("substitute.txt");
                    }
                }
                3 => {
                    if let api::RenameOutcome::Renamed { recovery, .. } = &mut value.outcome {
                        recovery.extend([
                            api::RenameRecovery {
                                role: api::RenameRecoveryRole::Source,
                                address: test_path::address(&retained_path),
                                identity: vec![0x99],
                            },
                            api::RenameRecovery {
                                role: api::RenameRecoveryRole::DisplacedTarget,
                                address: api::Address {
                                    encoding: api::AddressEncoding::Opaque,
                                    bytes: b"foreign-locator".to_vec(),
                                },
                                identity: vec![0x98],
                            },
                        ]);
                    }
                }
                _ => {}
            }
            if mode < 2
                && let api::RenameOutcome::Renamed { recovery, .. } = &mut value.outcome
            {
                recovery.push(api::RenameRecovery {
                    role: api::RenameRecoveryRole::Source,
                    address: test_path::address(&retained_path),
                    identity: vec![0x99],
                });
            }
        }
        let mut steps = opening(api::Kind::File, api::Kind::File);
        steps.push(Step::error(
            rename_request(1, "after.txt", api::RenamePolicy::NoReplace),
            api::ErrorCode::Io,
        ));
        let mut query = Step::ok(api::Request::GetRenameState { prepared: HANDLE }, reply);
        if mode == 4 {
            query.answer = Answer::Panic;
        }
        steps.push(query);
        if mode == 4 {
            steps.push(Step::error(
                api::Request::CloseRename { prepared: HANDLE },
                api::ErrorCode::Invalid,
            ));
        } else {
            steps.extend(closing());
        }
        let fixture = Fixture::new(steps);
        let resource = fixture.prepare();
        let requested = input(1, "after.txt", RenamePolicy::NoReplace);
        let value = resource.attempt(&requested, &Cancellation::default());
        assert_eq!(
            value.attempt, 1,
            "The submitted local input remains diagnostic context"
        );
        match &value.outcome {
            RenameOutcome::Unconfirmed {
                requested_name,
                last_known_attempt,
                recovery,
                ..
            } => {
                assert_eq!(requested_name, &OsString::from("after.txt"));
                assert_eq!(
                    *last_known_attempt, 0,
                    "No native attempt receipt was validated"
                );
                if mode < 2 {
                    assert!(
                        recovery.is_empty(),
                        "Recovery from another owner or future attempt was misattributed"
                    );
                }
                if mode == 3 {
                    assert_eq!(recovery.len(), 1);
                    assert_eq!(recovery[0].path, retained_path);
                    assert_eq!(recovery[0].role, midnight_core::RenameRecoveryRole::Source);
                    assert_eq!(recovery[0].identity, [0x99]);
                }
            }
            other => panic!("Untrusted query became a trusted receipt: {other:?}"),
        }
        assert!(matches!(
            resource.query().outcome,
            RenameOutcome::Unconfirmed { .. }
        ));
        let _ = resource.attempt(&requested, &Cancellation::default());
        assert_eq!(mutation_count(&fixture), 1);
        if mode == 4 {
            assert!(matches!(resource.finish(), RenameFinish::Unconfirmed(_)));
        } else {
            assert!(matches!(resource.finish(), RenameFinish::Closed));
        }
        assert!(
            matches!(resource.query().outcome, RenameOutcome::Unconfirmed { .. }),
            "Acknowledged disposal cannot rewrite the unknown mutation outcome"
        );
        drop(resource);
        fixture.check();
    }
}

#[test]
fn prepared_rename_finish_orders_closes_and_latches_permanent_failures() {
    for mode in 0..4 {
        let mut steps = opening(api::Kind::File, api::Kind::File);
        if mode == 2 {
            steps.push(Step::ok(
                rename_request(1, "after.txt", api::RenamePolicy::NoReplace),
                renamed(1, "after.txt"),
            ));
        }
        if mode < 2 {
            steps.push(Step::ok(
                api::Request::CloseRename { prepared: HANDLE },
                api::Response::Unit,
            ));
            steps.push(if mode == 0 {
                Step::ok(
                    api::Request::CloseMount { mount: MOUNT },
                    api::Response::Unit,
                )
            } else {
                Step::error(
                    api::Request::CloseMount { mount: MOUNT },
                    api::ErrorCode::Io,
                )
            });
        } else if mode == 2 {
            steps.push(Step::error(
                api::Request::CloseRename { prepared: HANDLE },
                api::ErrorCode::Busy,
            ));
        } else {
            steps.push(Step::ok(
                api::Request::CloseRename { prepared: HANDLE },
                api::Response::Failed(api::Failure::new(
                    api::ErrorCode::Busy,
                    "Busy without an associated actual call",
                )),
            ));
        }
        let fixture = Fixture::new(steps);
        let resource = fixture.prepare();
        if mode == 2 {
            assert_renamed(
                &resource.attempt(
                    &input(1, "after.txt", RenamePolicy::NoReplace),
                    &Cancellation::default(),
                ),
                1,
                "after.txt",
            );
        }
        for _ in 0..2 {
            if mode == 0 {
                assert!(matches!(resource.finish(), RenameFinish::Closed));
            } else {
                assert!(matches!(resource.finish(), RenameFinish::Unconfirmed(_)));
            }
        }
        let before = mutation_count(&fixture);
        let _ = resource.attempt(
            &input(2, "later.txt", RenamePolicy::NoReplace),
            &Cancellation::default(),
        );
        assert_eq!(
            mutation_count(&fixture),
            before,
            "finish must stop new mutation admission"
        );
        if mode == 2 {
            assert_renamed(&resource.query(), 1, "after.txt");
        }
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|(request, _)| matches!(request, api::Request::CloseRename { .. }))
                .count(),
            1
        );
        drop(resource);
        fixture.check();
    }
}

#[test]
fn prepared_rename_held_handler_allows_query_and_deferred_finish_without_deadlock() {
    let gate = Arc::new(CallGate::default());
    let mut steps = opening(api::Kind::File, api::Kind::File);
    steps.extend([
        Step::ok(
            rename_request(1, "after.txt", api::RenamePolicy::NoReplace),
            renamed(1, "after.txt"),
        )
        .held(&gate),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            state(1, api::RenameOutcome::Executing),
        ),
    ]);
    steps.extend(closing());
    let fixture = Fixture::new(steps);
    let resource = fixture.prepare();
    let cancellation = Cancellation::default();
    let requested = input(1, "after.txt", RenamePolicy::NoReplace);
    std::thread::scope(|scope| {
        // This guard opens the callback before scoped joins even on assertion unwind.
        let _release = ReleaseGate(gate.clone());
        let actual = scope.spawn(|| resource.attempt(&requested, &cancellation));
        gate.wait_entered();
        cancellation.cancel();
        let (send, received) = mpsc::sync_channel(1);
        let finishing = resource.clone();
        let finisher = scope.spawn(move || {
            let _ = send.send(finishing.finish());
        });
        assert!(matches!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("finish blocked behind the held mutation"),
            RenameFinish::Executing
        ));
        finisher.join().unwrap();
        assert!(!fixture.calls().iter().any(|(request, _)| matches!(
            request,
            api::Request::CloseRename { .. } | api::Request::CloseMount { .. }
        )));

        let (send, received) = mpsc::sync_channel(1);
        let querying = resource.clone();
        let query = scope.spawn(move || {
            let _ = send.send(querying.query());
        });
        assert!(matches!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("query blocked behind the held mutation")
                .outcome,
            RenameOutcome::Executing
        ));
        query.join().unwrap();

        let (send, received) = mpsc::sync_channel(1);
        let refusing = resource.clone();
        let later = scope.spawn(move || {
            let _ = send.send(refusing.attempt(
                &input(2, "later.txt", RenamePolicy::NoReplace),
                &Cancellation::default(),
            ));
        });
        let _ = received
            .recv_timeout(Duration::from_secs(2))
            .expect("Stopped resource waited for another mutation admission");
        later.join().unwrap();
        assert_eq!(mutation_count(&fixture), 1);
        gate.release();
        assert_renamed(&actual.join().unwrap(), 1, "after.txt");
    });
    assert_renamed(&resource.query(), 1, "after.txt");
    assert!(matches!(resource.finish(), RenameFinish::Closed));
    drop(resource);
    assert!(
        fixture
            .calls()
            .iter()
            .filter(|(request, _)| matches!(
                request,
                api::Request::GetRenameState { .. } | api::Request::CloseRename { .. }
            ))
            .all(|(_, cancelled)| !cancelled)
    );
    fixture.check();

    // Native execution can be complete while this host still needs its only
    // retained SDK receipt after lost delivery. Closing now would erase it.
    let recovery_gate = Arc::new(CallGate::default());
    let mut recovery_steps = opening(api::Kind::File, api::Kind::File);
    recovery_steps.extend([
        Step::error(
            rename_request(1, "recovered.txt", api::RenamePolicy::NoReplace),
            api::ErrorCode::Io,
        ),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            renamed(1, "recovered.txt"),
        )
        .held(&recovery_gate),
    ]);
    recovery_steps.extend(closing());
    let recovery_fixture = Fixture::new(recovery_steps);
    let recovering = recovery_fixture.prepare();
    let requested = input(1, "recovered.txt", RenamePolicy::NoReplace);
    std::thread::scope(|scope| {
        let _release = ReleaseGate(recovery_gate.clone());
        let actual = scope.spawn(|| recovering.attempt(&requested, &Cancellation::default()));
        recovery_gate.wait_entered();
        let (send, received) = mpsc::sync_channel(1);
        let finishing = recovering.clone();
        let finisher = scope.spawn(move || {
            let _ = send.send(finishing.finish());
        });
        assert!(matches!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("finish blocked during lost-reply recovery"),
            RenameFinish::Executing
        ));
        finisher.join().unwrap();
        assert!(
            !recovery_fixture.calls().iter().any(|(request, _)| matches!(
                request,
                api::Request::CloseRename { .. } | api::Request::CloseMount { .. }
            )),
            "CloseRename Unit would destroy the receipt still being recovered"
        );
        recovery_gate.release();
        assert_renamed(&actual.join().unwrap(), 1, "recovered.txt");
    });
    assert_renamed(&recovering.query(), 1, "recovered.txt");
    assert!(matches!(recovering.finish(), RenameFinish::Closed));
    drop(recovering);
    assert_eq!(mutation_count(&recovery_fixture), 1);
    recovery_fixture.check();
}

#[test]
fn prepared_rename_delayed_executing_query_cannot_erase_new_conflict_consent() {
    let actual_gate = Arc::new(CallGate::default());
    let query_gate = Arc::new(CallGate::default());
    let mut steps = opening(api::Kind::File, api::Kind::File);
    steps.extend([
        Step::ok(
            rename_request(1, "target.txt", api::RenamePolicy::NoReplace),
            conflict(1, 42, "target.txt"),
        )
        .held(&actual_gate),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            state(1, api::RenameOutcome::Executing),
        )
        .held(&query_gate),
        Step::ok(
            rename_request(
                2,
                "target.txt",
                api::RenamePolicy::RecoverableCurrentEntry { conflict: 42 },
            ),
            renamed(2, "target.txt"),
        ),
    ]);
    steps.extend(closing());
    let fixture = Fixture::new(steps);
    let resource = fixture.prepare();
    let token = std::thread::scope(|scope| {
        let _release_actual = ReleaseGate(actual_gate.clone());
        let _release_query = ReleaseGate(query_gate.clone());
        let (send, actual_received) = mpsc::sync_channel(1);
        let acting = resource.clone();
        let actual = scope.spawn(move || {
            let _ = send.send(acting.attempt(
                &input(1, "target.txt", RenamePolicy::NoReplace),
                &Cancellation::default(),
            ));
        });
        actual_gate.wait_entered();
        let (send, query_received) = mpsc::sync_channel(1);
        let querying = resource.clone();
        let query = scope.spawn(move || {
            let _ = send.send(querying.query());
        });
        query_gate.wait_entered();

        // Executing was captured earlier, but the authoritative Conflict is
        // accepted before that old query is allowed to return.
        actual_gate.release();
        let actual_state = actual_received
            .recv_timeout(Duration::from_secs(2))
            .expect("The actual Conflict waited for an older held query");
        let token = consent(&actual_state);
        actual.join().unwrap();
        query_gate.release();
        let observed = query_received
            .recv_timeout(Duration::from_secs(2))
            .expect("The delayed query did not return");
        assert_eq!(
            consent(&observed),
            token,
            "Old Executing erased current consent"
        );
        query.join().unwrap();
        token
    });
    assert_eq!(mutation_count(&fixture), 1);
    assert_renamed(
        &resource.attempt(
            &input(
                2,
                "target.txt",
                RenamePolicy::RecoverableCurrentEntry { conflict: token },
            ),
            &Cancellation::default(),
        ),
        2,
        "target.txt",
    );
    assert_eq!(
        mutation_count(&fixture),
        2,
        "Only explicit Replace may send another operation21"
    );
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|(request, _)| matches!(request, api::Request::GetRenameState { .. }))
            .count(),
        1
    );
    assert!(matches!(resource.finish(), RenameFinish::Closed));
    drop(resource);
    fixture.check();
}

#[test]
fn prepared_rename_failed_inflight_query_cannot_override_actual_renamed_result() {
    let actual_gate = Arc::new(CallGate::default());
    let mut steps = opening(api::Kind::File, api::Kind::File);
    steps.extend([
        Step::ok(
            rename_request(1, "after.txt", api::RenamePolicy::NoReplace),
            renamed(1, "after.txt"),
        )
        .held(&actual_gate),
        Step::error(
            api::Request::GetRenameState { prepared: HANDLE },
            api::ErrorCode::Io,
        ),
    ]);
    steps.extend(closing());
    let fixture = Fixture::new(steps);
    let resource = fixture.prepare();
    std::thread::scope(|scope| {
        let _release_actual = ReleaseGate(actual_gate.clone());
        let (send, actual_received) = mpsc::sync_channel(1);
        let acting = resource.clone();
        let actual = scope.spawn(move || {
            let _ = send.send(acting.attempt(
                &input(1, "after.txt", RenamePolicy::NoReplace),
                &Cancellation::default(),
            ));
        });
        actual_gate.wait_entered();
        let (send, query_received) = mpsc::sync_channel(1);
        let querying = resource.clone();
        let query = scope.spawn(move || {
            let _ = send.send(querying.query());
        });
        let provisional = query_received
            .recv_timeout(Duration::from_secs(2))
            .expect("The failed query waited for the held actual mutation");
        assert!(matches!(
            provisional.outcome,
            RenameOutcome::Unconfirmed { .. }
        ));
        query.join().unwrap();
        assert_eq!(mutation_count(&fixture), 1);

        actual_gate.release();
        let actual_state = actual_received
            .recv_timeout(Duration::from_secs(2))
            .expect("The authoritative Rename result did not return");
        assert_renamed(&actual_state, 1, "after.txt");
        actual.join().unwrap();
    });
    assert_renamed(&resource.query(), 1, "after.txt");
    assert_eq!(mutation_count(&fixture), 1);
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|(request, _)| matches!(request, api::Request::GetRenameState { .. }))
            .count(),
        1
    );
    assert!(matches!(resource.finish(), RenameFinish::Closed));
    drop(resource);
    fixture.check();
}

#[test]
fn prepared_rename_failed_inflight_query_still_recovers_lost_actual_reply() {
    let actual_gate = Arc::new(CallGate::default());
    let mut steps = opening(api::Kind::File, api::Kind::File);
    steps.extend([
        Step::error(
            rename_request(1, "after.txt", api::RenamePolicy::NoReplace),
            api::ErrorCode::Io,
        )
        .held(&actual_gate),
        Step::error(
            api::Request::GetRenameState { prepared: HANDLE },
            api::ErrorCode::Io,
        ),
        Step::ok(
            api::Request::GetRenameState { prepared: HANDLE },
            renamed(1, "after.txt"),
        ),
    ]);
    steps.extend(closing());
    let fixture = Fixture::new(steps);
    let resource = fixture.prepare();
    std::thread::scope(|scope| {
        let _release_actual = ReleaseGate(actual_gate.clone());
        let (send, actual_received) = mpsc::sync_channel(1);
        let acting = resource.clone();
        let actual = scope.spawn(move || {
            let _ = send.send(acting.attempt(
                &input(1, "after.txt", RenamePolicy::NoReplace),
                &Cancellation::default(),
            ));
        });
        actual_gate.wait_entered();
        let (send, query_received) = mpsc::sync_channel(1);
        let querying = resource.clone();
        let query = scope.spawn(move || {
            let _ = send.send(querying.query());
        });
        let provisional = query_received
            .recv_timeout(Duration::from_secs(2))
            .expect("The failed external query waited for actual operation21");
        assert!(matches!(
            provisional.outcome,
            RenameOutcome::Unconfirmed { .. }
        ));
        query.join().unwrap();

        let (send, refused_received) = mpsc::sync_channel(1);
        let refusing = resource.clone();
        let refused = scope.spawn(move || {
            let _ = send.send(refusing.attempt(
                &input(2, "later.txt", RenamePolicy::NoReplace),
                &Cancellation::default(),
            ));
        });
        let _ = refused_received
            .recv_timeout(Duration::from_secs(2))
            .expect("Blocked mutation admission waited on the actual callback");
        refused.join().unwrap();
        assert_eq!(mutation_count(&fixture), 1);

        // The actual reply is also lost. Its handler must still perform the
        // independent recovery query despite the earlier failed observation.
        actual_gate.release();
        let recovered = actual_received
            .recv_timeout(Duration::from_secs(2))
            .expect("The actual lost reply did not finish receipt recovery");
        assert_renamed(&recovered, 1, "after.txt");
        actual.join().unwrap();
    });
    assert_renamed(&resource.query(), 1, "after.txt");
    let _ = resource.attempt(
        &input(2, "later.txt", RenamePolicy::NoReplace),
        &Cancellation::default(),
    );
    assert_eq!(mutation_count(&fixture), 1);
    let queries: Vec<_> = fixture
        .calls()
        .into_iter()
        .filter(|(request, _)| matches!(request, api::Request::GetRenameState { .. }))
        .collect();
    assert_eq!(
        queries.len(),
        2,
        "External observation and actual-handler recovery are distinct"
    );
    assert!(queries.iter().all(|(_, cancelled)| !cancelled));
    assert!(matches!(resource.finish(), RenameFinish::Closed));
    drop(resource);
    fixture.check();
}

#[test]
fn prepared_rename_queued_producer_cancellation_has_no_unknown_publication() {
    for queued_prepare in [false, true] {
        let cancellation = Cancellation::default();
        let steps = if queued_prepare {
            let mut steps = opening(api::Kind::File, api::Kind::File);
            steps[1] = Step::error(
                api::Request::PrepareRename {
                    mount: MOUNT,
                    node: NODE,
                },
                api::ErrorCode::Cancelled,
            )
            .cancel(&cancellation);
            steps.push(Step::ok(
                api::Request::CloseMount { mount: MOUNT },
                api::Response::Unit,
            ));
            steps
        } else {
            vec![
                Step::error(
                    api::Request::Resolve {
                        address: test_path::address(&source_path()),
                    },
                    api::ErrorCode::Cancelled,
                )
                .cancel(&cancellation),
            ]
        };
        let fixture = Fixture::new(steps);
        // Private Transport Err(Cancelled) proves its admission was cancelled
        // before a native callback. Provider refusals use encoded Failed instead.
        let failure = preparation_failure(&fixture, &cancellation);
        assert!(cancellation.is_cancelled());
        assert_eq!(failure.error.kind, std::io::ErrorKind::Interrupted);
        assert!(
            failure.retained.is_none(),
            "No newly published native resource can remain"
        );
        let calls = fixture.calls();
        assert!(
            !calls
                .iter()
                .any(|(request, _)| matches!(request, api::Request::CloseRename { .. }))
        );
        assert_eq!(
            calls
                .iter()
                .filter(|(request, _)| matches!(request, api::Request::CloseMount { .. }))
                .count(),
            usize::from(queued_prepare)
        );
        if queued_prepare {
            assert!(matches!(
                calls.last(),
                Some((api::Request::CloseMount { mount: MOUNT }, false))
            ));
            assert_eq!(calls.len(), 3);
        } else {
            assert_eq!(calls.len(), 1);
        }
        fixture.check();

        // Raw protocol failure has no matching pre-dispatch guarantee. Keep
        // publication uncertainty even though Cancelled used the same producer.
        let mut invalid_steps = if queued_prepare {
            let mut steps = opening(api::Kind::File, api::Kind::File);
            steps.pop();
            steps
        } else {
            vec![]
        };
        let invalid_request = if queued_prepare {
            api::Request::PrepareRename {
                mount: MOUNT,
                node: NODE,
            }
        } else {
            api::Request::Resolve {
                address: test_path::address(&source_path()),
            }
        };
        invalid_steps.push(Step::error(invalid_request, api::ErrorCode::Invalid));
        let invalid_fixture = Fixture::new(invalid_steps);
        let failure = preparation_failure(&invalid_fixture, &Cancellation::default());
        let retained = failure
            .retained
            .expect("Raw Invalid may have published hidden resources");
        assert!(matches!(retained.finish(), RenameFinish::Unconfirmed(_)));
        drop(retained);
        assert!(!invalid_fixture.calls().iter().any(|(request, _)| matches!(
            request,
            api::Request::CloseRename { .. } | api::Request::CloseMount { .. }
        )));
        invalid_fixture.check();
    }
}
