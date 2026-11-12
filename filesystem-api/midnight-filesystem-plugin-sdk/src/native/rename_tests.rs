//! Controlled namespace conformance. No native I/O or real mutation qualification.
use super::*;
use crate::api::abi::*;
use crate::ffi_tests::{create_instance, descriptor, destroy, invoke, read, release};
use std::{
    collections::HashMap,
    sync::{
        Condvar, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Namespace {
    object: AtomicU64,
    parent: AtomicU64,
    target: AtomicU64,
    target_revision: AtomicU64,
    can_replace: AtomicBool,
    recoverable: AtomicBool,
    recovered_targets: Mutex<Vec<Vec<u8>>>,
    size: AtomicU64,
    kind: Mutex<Option<Kind>>,
    pin_kind: Mutex<Option<Kind>>,
    prepares: AtomicUsize,
    executes: AtomicUsize,
    writes: AtomicUsize,
    drops: AtomicUsize,
    fail_binding: AtomicBool,
    cancel: AtomicBool,
    mode: AtomicUsize,
    gate: Mutex<Option<Arc<Gate>>>,
    prepare_gate: Mutex<Option<Arc<Gate>>>,
    drop_gate: Mutex<Option<Arc<Gate>>>,
}
#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl Gate {
    fn enter(&self) {
        let mut state = lock(&self.state);
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }
    fn wait_entered(&self) {
        let state = lock(&self.state);
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |s| !s.0)
            .unwrap();
        assert!(state.0, "execution did not enter its controlled gate");
    }
    fn unblock(&self) {
        lock(&self.state).1 = true;
        self.changed.notify_all();
    }
}
struct ReleaseGate(Arc<Gate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.unblock();
    }
}
static FIXTURES: OnceLock<Mutex<HashMap<PathBuf, Arc<Namespace>>>> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
fn fixtures() -> &'static Mutex<HashMap<PathBuf, Arc<Namespace>>> {
    FIXTURES.get_or_init(Default::default)
}
fn namespace(path: &Path) -> Arc<Namespace> {
    let values = lock(fixtures());
    values
        .get(path)
        .or_else(|| path.parent().and_then(|p| values.get(p)))
        .unwrap()
        .clone()
}
struct Adapter;
struct Directory(Arc<Namespace>, usize);
struct Reader;
impl NativeReader for Reader {
    fn length(&self) -> u64 {
        0
    }
    fn revision(&self) -> &[u8] {
        b"reader"
    }
    fn validate(&self, _: &CallContext<'_>) -> Result<(), Failure> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32, _: &CallContext<'_>) -> Result<Vec<u8>, Failure> {
        Ok(vec![])
    }
}
fn text(value: &[u8]) -> NativeText {
    NativeText {
        encoding: NameEncoding::Utf8,
        bytes: value.to_vec(),
    }
}
fn object(ns: &Namespace, kind: Kind) -> RenameObject {
    RenameObject {
        identity: ns.object.load(Ordering::Acquire).to_le_bytes().to_vec(),
        revision: b"pinned-binding".to_vec(),
        name: text(b"entry0"),
        kind,
        metadata: Metadata {
            size: (kind == Kind::File).then(|| ns.size.load(Ordering::Acquire)),
            ..Default::default()
        },
    }
}
impl NativeAdapter for Adapter {
    type Directory = Directory;
    type Reader = Reader;
    fn decode(value: &Address) -> Result<PathBuf, Failure> {
        Ok(PathBuf::from(std::str::from_utf8(&value.bytes).unwrap()))
    }
    fn encode(path: &Path) -> Address {
        Address {
            encoding: AddressEncoding::Opaque,
            bytes: path.to_str().unwrap().as_bytes().to_vec(),
        }
    }
    fn name(name: &OsStr) -> (NameEncoding, Vec<u8>) {
        (
            NameEncoding::Utf8,
            name.to_str().unwrap().as_bytes().to_vec(),
        )
    }
    fn stat(path: &Path, _: &CallContext<'_>) -> Result<NativeInfo, Failure> {
        let ns = namespace(path);
        let root = lock(fixtures()).contains_key(path);
        let kind = if root {
            Kind::Directory
        } else {
            lock(&ns.kind).unwrap_or(Kind::File)
        };
        Ok(NativeInfo {
            kind,
            metadata: object(&ns, kind).metadata,
        })
    }
    fn list(path: &Path, _: &CallContext<'_>) -> Result<Directory, Failure> {
        Ok(Directory(namespace(path), 0))
    }
    fn next(
        directory: &mut Directory,
        _: &CallContext<'_>,
    ) -> Result<Option<NativeEntry>, Failure> {
        if directory.1 >= 128 {
            return Ok(None);
        }
        let name = format!("entry{}", directory.1);
        directory.1 += 1;
        let kind = lock(&directory.0.kind).unwrap_or(Kind::File);
        Ok(Some(NativeEntry {
            name: name.into(),
            info: NativeInfo {
                kind,
                metadata: object(&directory.0, kind).metadata,
            },
        }))
    }
    fn open_reader(_: &Path, _: &CallContext<'_>) -> Result<Reader, Failure> {
        Ok(Reader)
    }
    fn volumes(_: &CallContext<'_>) -> Result<Vec<Volume>, Failure> {
        Ok(vec![])
    }
    fn storage_kind(_: &Path, _: &CallContext<'_>) -> Result<StorageKind, Failure> {
        Ok(StorageKind::Unknown)
    }
    fn space_info(_: &Path, _: &CallContext<'_>) -> Result<(u64, u64), Failure> {
        Ok((1, 2))
    }
    fn prepare_rename(path: &Path, _: &CallContext<'_>) -> Result<Arc<dyn NativeRename>, Failure> {
        let ns = namespace(path);
        ns.prepares.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = lock(&ns.prepare_gate).clone() {
            gate.enter();
        }
        if ns.fail_binding.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::SourceChanged));
        }
        let kind = lock(&ns.pin_kind).or(*lock(&ns.kind)).unwrap_or(Kind::File);
        let mut info = PreparedRenameInfo {
            source: object(&ns, kind),
            parent_identity: ns.parent.load(Ordering::Acquire).to_le_bytes().to_vec(),
            recoverable_replace: kind == Kind::File && ns.recoverable.load(Ordering::Acquire),
        };
        if ns.mode.load(Ordering::Acquire) == 9 {
            info.source.name = text(b"substituted-name");
        }
        if ns.mode.load(Ordering::Acquire) == 7 {
            ns.cancel.store(true, Ordering::Release);
        }
        Ok(Arc::new(Pin {
            ns,
            info,
            conflict: Mutex::default(),
        }))
    }
}
struct Pin {
    ns: Arc<Namespace>,
    info: PreparedRenameInfo,
    conflict: Mutex<Option<RenameObject>>,
}
impl Drop for Pin {
    fn drop(&mut self) {
        if let Some(gate) = lock(&self.ns.drop_gate).clone() {
            gate.enter();
        }
        self.ns.drops.fetch_add(1, Ordering::AcqRel);
    }
}
impl NativeRename for Pin {
    fn prepared(&self) -> PreparedRenameInfo {
        self.info.clone()
    }
    fn execute(
        &self,
        name: &NativeText,
        policy: &RenamePolicy,
        context: &CallContext<'_>,
    ) -> RenameOutcome {
        self.ns.executes.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = lock(&self.ns.gate).clone() {
            gate.enter();
        }
        if self.info.source.identity != self.ns.object.load(Ordering::Acquire).to_le_bytes()
            || self.info.parent_identity != self.ns.parent.load(Ordering::Acquire).to_le_bytes()
        {
            return RenameOutcome::Unchanged(failure(ErrorCode::SourceChanged));
        }
        if context.is_cancelled() {
            return RenameOutcome::Unchanged(failure(ErrorCode::Cancelled));
        }
        match self.ns.mode.load(Ordering::Acquire) {
            1 => {
                let target = RenameObject {
                    identity: self
                        .ns
                        .target
                        .load(Ordering::Acquire)
                        .to_le_bytes()
                        .to_vec(),
                    revision: self
                        .ns
                        .target_revision
                        .load(Ordering::Acquire)
                        .to_le_bytes()
                        .to_vec(),
                    name: name.clone(),
                    kind: Kind::File,
                    metadata: Metadata {
                        size: Some(89),
                        ..Default::default()
                    },
                };
                let previous = lock(&self.conflict).replace(target.clone());
                let can_replace = self.ns.can_replace.load(Ordering::Acquire)
                    && self.info.recoverable_replace
                    && target.identity != self.info.source.identity;
                if matches!(policy, RenamePolicy::NoReplace)
                    || previous.as_ref() != Some(&target)
                    || !can_replace
                {
                    RenameOutcome::Conflict(RenameConflict {
                        token: 0,
                        target,
                        can_replace,
                    })
                } else {
                    self.ns.writes.fetch_add(1, Ordering::AcqRel);
                    lock(&self.ns.recovered_targets).push(target.identity.clone());
                    RenameOutcome::Renamed {
                        name: name.clone(),
                        same_entry: false,
                        recovery: vec![RenameRecovery {
                            role: RenameRecoveryRole::DisplacedTarget,
                            address: Address {
                                encoding: AddressEncoding::Opaque,
                                bytes: b"fixture:displaced-target".to_vec(),
                            },
                            identity: target.identity,
                        }],
                        durability_confirmed: true,
                    }
                }
            }
            2 => RenameOutcome::Ready, // Invalid author result: outcome must become unknown.
            3 => RenameOutcome::Unchanged(failure(ErrorCode::PermissionDenied)),
            4 => {
                self.ns.writes.fetch_add(1, Ordering::AcqRel);
                RenameOutcome::RecoveryRequired {
                    source_location: RenameSourceLocation::Recovery,
                    destination_name: None,
                    recovery: vec![RenameRecovery {
                        role: RenameRecoveryRole::Source,
                        address: Address {
                            encoding: AddressEncoding::UnixBytes,
                            bytes: b"/fixture/.stage/source-\xff".to_vec(),
                        },
                        identity: self.info.source.identity.clone(),
                    }],
                    error: failure(ErrorCode::Io),
                }
            }
            _ => {
                self.ns.writes.fetch_add(1, Ordering::AcqRel);
                if self.ns.mode.load(Ordering::Acquire) == 5 {
                    self.ns.cancel.store(true, Ordering::Release);
                }
                RenameOutcome::Renamed {
                    name: name.clone(),
                    same_entry: false,
                    recovery: vec![],
                    durability_confirmed: true,
                }
            }
        }
    }
}
fn call(api: &Api, instance: u64, request: Request) -> Response {
    let (status, buffer) = invoke(api, instance, request, Cancellation::default());
    assert_eq!(status, OK);
    let response = std::panic::catch_unwind(|| read(buffer));
    assert_eq!(release(api, instance, buffer.id), OK);
    response.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
struct Fixture {
    api: Api,
    instance: u64,
    root: PathBuf,
    ns: Arc<Namespace>,
    mount: u64,
    root_node: u64,
    node: u64,
    cursor: u64,
}
impl Fixture {
    fn new() -> Self {
        Self::with_config(Config::default())
    }
    fn with_config(config: Config) -> Self {
        let root = std::env::current_dir().unwrap().join(format!(
            "rename-fake-{}",
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let ns = Arc::new(Namespace::default());
        ns.object.store(1, Ordering::Release);
        ns.parent.store(10, Ordering::Release);
        ns.target.store(100, Ordering::Release);
        ns.target_revision.store(1, Ordering::Release);
        ns.can_replace.store(true, Ordering::Release);
        ns.recoverable.store(true, Ordering::Release);
        ns.size.store(7, Ordering::Release);
        lock(fixtures()).insert(root.clone(), ns.clone());
        let api = descriptor::<NativeFilesystem<Adapter>>();
        let instance = create_instance(&api, config);
        let Response::Resolved {
            mount,
            node: root_node,
        } = call(
            &api,
            instance,
            Request::Resolve {
                address: Adapter::encode(&root),
            },
        )
        else {
            panic!("resolve")
        };
        let Response::Page {
            cursor, entries, ..
        } = call(
            &api,
            instance,
            Request::ListBegin {
                mount,
                directory: root_node.id,
                max_entries: 2,
            },
        )
        else {
            panic!("list")
        };
        Self {
            api,
            instance,
            root,
            ns,
            mount,
            root_node: root_node.id,
            node: entries[0].id,
            cursor,
        }
    }
    fn prepare(&self) -> Response {
        self.call(Request::PrepareRename {
            mount: self.mount,
            node: self.node,
        })
    }
    fn pin(&self) -> PreparedRename {
        let Response::RenamePrepared(prepared) = self.prepare() else {
            panic!("prepare")
        };
        prepared
    }
    fn call(&self, request: Request) -> Response {
        call(&self.api, self.instance, request)
    }
    fn attempt(&self, prepared: u64, attempt: u64, name: &[u8], policy: RenamePolicy) -> Response {
        self.call(Request::Rename {
            prepared,
            attempt,
            name: text(name),
            policy,
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert_eq!(destroy(&self.api, self.instance), OK);
        lock(fixtures()).remove(&self.root);
    }
}
fn code(response: Response, expected: ErrorCode) {
    assert!(
        matches!(response, Response::Failed(Failure { code, .. }) if code == expected),
        "{response:?}"
    );
}
fn state(response: Response) -> RenameState {
    let Response::RenameState(state) = response else {
        panic!("state: {response:?}")
    };
    state
}

#[test]
fn browse_locator_prepares_current_same_kind_object() {
    let f = Fixture::new();
    f.ns.object.store(2, Ordering::Release);
    f.ns.size.store(123, Ordering::Release);
    let Response::Page {
        cursor, entries, ..
    } = f.call(Request::ListBegin {
        mount: f.mount,
        directory: f.root_node,
        max_entries: 1,
    })
    else {
        panic!("list")
    };
    assert_eq!(
        entries[0].id, f.node,
        "the logical path/kind ID remains reusable"
    );
    let pin = f.pin();
    assert_eq!(pin.source.identity, 2u64.to_le_bytes());
    assert_eq!(pin.source.metadata.size, Some(123));
    assert_eq!(f.ns.prepares.load(Ordering::Acquire), 1);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
    assert_eq!(f.call(Request::CloseList { cursor }), Response::Unit);
}
#[test]
fn browse_metadata_is_not_an_identity_check() {
    let f = Fixture::new();
    f.ns.object.store(99, Ordering::Release); // Same name, kind, size and timestamps.
    let pin = f.pin();
    assert_eq!(pin.source.identity, 99u64.to_le_bytes());
    assert_eq!(pin.source.metadata.size, Some(7));
}
#[test]
fn prepare_checks_pinned_kind_before_publication() {
    let f = Fixture::new();
    *lock(&f.ns.pin_kind) = Some(Kind::Directory);
    for _ in 0..40 {
        code(f.prepare(), ErrorCode::SourceChanged);
    }
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 40);
    *lock(&f.ns.pin_kind) = None;
    assert_ne!(
        f.pin().handle,
        0,
        "failed preparation returns count and byte admission"
    );
}
#[test]
fn link_navigation_hints_do_not_turn_a_pin_into_referent_authority() {
    let f = Fixture::new();
    *lock(&f.ns.kind) = Some(Kind::DirectoryLink);
    let Response::Page { entries, .. } = f.call(Request::ListBegin {
        mount: f.mount,
        directory: f.root_node,
        max_entries: 1,
    }) else {
        panic!("list")
    };
    *lock(&f.ns.pin_kind) = Some(Kind::Symlink);
    let Response::RenamePrepared(pin) = f.call(Request::PrepareRename {
        mount: f.mount,
        node: entries[0].id,
    }) else {
        panic!("link prepare")
    };
    assert_eq!(pin.source.kind, Kind::Symlink);
    assert_eq!(f.ns.prepares.load(Ordering::Acquire), 1);
}
#[test]
fn failed_binding_validation_does_not_reprepare_a_new_object() {
    let f = Fixture::new();
    f.ns.fail_binding.store(true, Ordering::Release);
    code(f.prepare(), ErrorCode::SourceChanged);
    assert_eq!(f.ns.prepares.load(Ordering::Acquire), 1);
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 0);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
}
#[test]
fn prepared_identity_survives_confirmation_and_is_not_repinned() {
    for replace_parent in [false, true] {
        let f = Fixture::new();
        let pin = f.pin();
        if replace_parent {
            f.ns.parent.store(11, Ordering::Release);
        } else {
            f.ns.object.store(2, Ordering::Release);
        }
        let result = state(f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace));
        assert!(matches!(
            result.outcome,
            RenameOutcome::Unchanged(Failure {
                code: ErrorCode::SourceChanged,
                ..
            })
        ));
        assert_eq!(
            f.call(Request::GetRenameState {
                prepared: pin.handle
            }),
            Response::RenameState(result)
        );
        assert_eq!(f.ns.prepares.load(Ordering::Acquire), 1);
        assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
    }
}
#[test]
fn enumeration_acquires_no_rename_pins() {
    let f = Fixture::new();
    let _ = f.call(Request::ListNext {
        cursor: f.cursor,
        expected_page: 1,
        max_entries: 126,
    });
    assert_eq!(f.ns.prepares.load(Ordering::Acquire), 0);
    f.pin();
    assert_eq!(f.ns.prepares.load(Ordering::Acquire), 1);
}
#[test]
fn provider_epoch_staleness_is_distinct_from_external_replacement() {
    let f = Fixture::new();
    let pin = f.pin();
    let result = state(f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace));
    assert!(matches!(result.outcome, RenameOutcome::Renamed { .. }));
    code(
        f.call(Request::Stat {
            mount: f.mount,
            node: f.node,
        }),
        ErrorCode::StaleReference,
    );
    code(
        f.call(Request::ListNext {
            cursor: f.cursor,
            expected_page: 1,
            max_entries: 1,
        }),
        ErrorCode::StaleReference,
    );
    code(f.prepare(), ErrorCode::StaleReference);
    assert_eq!(
        f.call(Request::GetRenameState {
            prepared: pin.handle
        }),
        Response::RenameState(result)
    );
    assert_eq!(
        f.call(Request::CloseRename {
            prepared: pin.handle
        }),
        Response::Unit
    );
}
#[test]
fn close_mount_preserves_busy_with_live_rename_owner() {
    let f = Fixture::new();
    let pin = f.pin();
    assert_eq!(
        f.call(Request::CloseList { cursor: f.cursor }),
        Response::Unit
    );
    code(
        f.call(Request::CloseMount { mount: f.mount }),
        ErrorCode::Busy,
    );
    assert_eq!(
        f.call(Request::CloseRename {
            prepared: pin.handle
        }),
        Response::Unit
    );
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 1);
    assert_eq!(
        f.call(Request::CloseMount { mount: f.mount }),
        Response::Unit
    );
    code(f.prepare(), ErrorCode::StaleReference);
    assert_eq!(f.ns.prepares.load(Ordering::Acquire), 1);
}
#[test]
fn exactly_thirty_two_live_pins_are_admitted_and_close_returns_the_slot() {
    let f = Fixture::new();
    let pins: Vec<_> = (0..32).map(|_| f.pin()).collect();
    code(f.prepare(), ErrorCode::ResourceLimit);
    assert_eq!(
        f.ns.prepares.load(Ordering::Acquire),
        32,
        "reserve admission before opening native pins"
    );
    assert_eq!(
        f.call(Request::CloseRename {
            prepared: pins[0].handle
        }),
        Response::Unit
    );
    assert_ne!(f.pin().handle, pins[0].handle);
}
#[test]
fn foreign_typed_and_closed_handles_cannot_execute_or_query() {
    let f = Fixture::new();
    let other = Fixture::new();
    let pin = f.pin();
    let Response::Opened { reader, .. } = f.call(Request::OpenRead {
        mount: f.mount,
        node: f.node,
    }) else {
        panic!("reader")
    };
    for id in [f.mount, f.node, f.cursor, reader, other.pin().handle] {
        code(
            f.call(Request::GetRenameState { prepared: id }),
            ErrorCode::StaleReference,
        );
        code(
            f.attempt(id, 1, b"renamed", RenamePolicy::NoReplace),
            ErrorCode::StaleReference,
        );
    }
    code(
        other.call(Request::GetRenameState {
            prepared: pin.handle,
        }),
        ErrorCode::StaleReference,
    );
    f.call(Request::CloseRename {
        prepared: pin.handle,
    });
    code(
        f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace),
        ErrorCode::StaleReference,
    );
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 0);
}
#[test]
fn duplicate_attempt_replays_but_changed_payload_and_terminal_reexecution_refuse() {
    let f = Fixture::new();
    let pin = f.pin();
    assert_eq!(
        state(f.call(Request::GetRenameState {
            prepared: pin.handle
        }))
        .outcome,
        RenameOutcome::Ready
    );
    let result = f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace);
    assert_eq!(
        f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace),
        result
    );
    code(
        f.attempt(pin.handle, 1, b"different", RenamePolicy::NoReplace),
        ErrorCode::Invalid,
    );
    code(
        f.attempt(pin.handle, 2, b"renamed", RenamePolicy::NoReplace),
        ErrorCode::StaleReference,
    );
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 1);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
}
#[test]
fn conflicts_require_exact_name_token_and_next_attempt_and_changed_target_gets_new_token() {
    let f = Fixture::new();
    let pin = f.pin();
    f.ns.mode.store(1, Ordering::Release);
    code(
        f.attempt(pin.handle, 2, b"target", RenamePolicy::NoReplace),
        ErrorCode::Invalid,
    );
    let RenameOutcome::Conflict(first) =
        state(f.attempt(pin.handle, 1, b"target", RenamePolicy::NoReplace)).outcome
    else {
        panic!("conflict")
    };
    assert_ne!(first.token, 0);
    assert_eq!(first.target.name, text(b"target"));
    assert_ne!(first.target.identity, pin.source.identity);
    f.ns.target.store(101, Ordering::Release);
    f.ns.target_revision.store(2, Ordering::Release);
    code(
        f.attempt(
            pin.handle,
            2,
            b"target",
            RenamePolicy::RecoverableCurrentEntry { conflict: f.mount },
        ),
        ErrorCode::Invalid,
    );
    code(
        f.attempt(
            pin.handle,
            2,
            b"other",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first.token,
            },
        ),
        ErrorCode::Invalid,
    );
    let RenameOutcome::Conflict(second) = state(f.attempt(
        pin.handle,
        2,
        b"target",
        RenamePolicy::RecoverableCurrentEntry {
            conflict: first.token,
        },
    ))
    .outcome
    else {
        panic!("fresh conflict")
    };
    assert_ne!(first.token, second.token);
    assert_ne!(first.target.identity, second.target.identity);
    assert_ne!(first.target.revision, second.target.revision);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
    code(
        f.attempt(pin.handle, 1, b"target", RenamePolicy::NoReplace),
        ErrorCode::StaleReference,
    );
    code(
        f.attempt(
            pin.handle,
            3,
            b"target",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first.token,
            },
        ),
        ErrorCode::Invalid,
    );
    f.ns.mode.store(0, Ordering::Release);
    assert!(matches!(
        state(f.attempt(pin.handle, 3, b"other", RenamePolicy::NoReplace)).outcome,
        RenameOutcome::Renamed { .. }
    ));
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 3);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
}
#[test]
fn executing_query_is_read_only_and_close_and_concurrent_execution_are_busy() {
    let f = Fixture::new();
    let pin = f.pin();
    let gate = Arc::new(Gate::default());
    *lock(&f.ns.gate) = Some(gate.clone());
    std::thread::scope(|scope| {
        let _release = ReleaseGate(gate.clone());
        let api = &f.api;
        let instance = f.instance;
        // Api contains borrowed immutable descriptor pointers; transport this call
        // through a fresh descriptor rather than sharing raw pointers across threads.
        let running = scope.spawn(move || {
            let api = descriptor::<NativeFilesystem<Adapter>>();
            call(
                &api,
                instance,
                Request::Rename {
                    prepared: pin.handle,
                    attempt: 1,
                    name: text(b"renamed"),
                    policy: RenamePolicy::NoReplace,
                },
            )
        });
        gate.wait_entered();
        assert_eq!(
            state(call(
                api,
                instance,
                Request::GetRenameState {
                    prepared: pin.handle
                }
            ))
            .outcome,
            RenameOutcome::Executing
        );
        code(
            call(
                api,
                instance,
                Request::CloseRename {
                    prepared: pin.handle,
                },
            ),
            ErrorCode::Busy,
        );
        code(
            call(
                api,
                instance,
                Request::Rename {
                    prepared: pin.handle,
                    attempt: 1,
                    name: text(b"renamed"),
                    policy: RenamePolicy::NoReplace,
                },
            ),
            ErrorCode::Busy,
        );
        assert_eq!(
            f.ns.drops.load(Ordering::Acquire),
            0,
            "Busy close must retain the native pin"
        );
        gate.unblock();
        assert!(matches!(
            state(running.join().unwrap()).outcome,
            RenameOutcome::Renamed { .. }
        ));
    });
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 1);
    assert_eq!(
        f.call(Request::CloseRename {
            prepared: pin.handle
        }),
        Response::Unit
    );
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 1);
}
#[test]
fn publication_failure_keeps_the_completed_receipt_and_never_reexecutes() {
    let f = Fixture::new();
    let pin = f.pin();
    crate::runtime::fail_next_publish::<NativeFilesystem<Adapter>>(f.instance);
    let (status, buffer) = invoke(
        &f.api,
        f.instance,
        Request::Rename {
            prepared: pin.handle,
            attempt: 1,
            name: text(b"renamed"),
            policy: RenamePolicy::NoReplace,
        },
        Cancellation::default(),
    );
    assert_eq!(status, RESOURCE_LIMIT);
    assert_eq!(buffer.id, 0);
    let result = state(f.call(Request::GetRenameState {
        prepared: pin.handle,
    }));
    assert!(matches!(result.outcome, RenameOutcome::Renamed { .. }));
    assert_eq!(
        f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace),
        Response::RenameState(result)
    );
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
}
// SAFETY: each invocation retains the fixture's Namespace Arc until callback return.
unsafe extern "C" fn cancelled(context: u64) -> u32 {
    let ns = unsafe { &*(context as *const Namespace) };
    u32::from(ns.cancel.load(Ordering::Acquire))
}
fn cancellation(f: &Fixture) -> Cancellation {
    Cancellation {
        context: Arc::as_ptr(&f.ns) as u64,
        cancelled: Some(cancelled),
    }
}
#[test]
fn cancellation_after_commit_returns_actual_result_and_control_calls_ignore_cancellation() {
    let f = Fixture::new();
    let pin = f.pin();
    f.ns.mode.store(5, Ordering::Release);
    let (status, buffer) = invoke(
        &f.api,
        f.instance,
        Request::Rename {
            prepared: pin.handle,
            attempt: 1,
            name: text(b"renamed"),
            policy: RenamePolicy::NoReplace,
        },
        cancellation(&f),
    );
    assert_eq!(status, OK);
    let result = read(buffer);
    assert!(matches!(
        state(result.clone()).outcome,
        RenameOutcome::Renamed { .. }
    ));
    assert_eq!(release(&f.api, f.instance, buffer.id), OK);
    for request in [
        Request::GetRenameState {
            prepared: pin.handle,
        },
        Request::CloseRename {
            prepared: pin.handle,
        },
    ] {
        let (status, buffer) = invoke(&f.api, f.instance, request, cancellation(&f));
        assert_eq!(status, OK);
        assert!(!matches!(read(buffer), Response::Failed(_)));
        assert_eq!(release(&f.api, f.instance, buffer.id), OK);
    }
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
}
#[test]
fn prepare_late_cancellation_and_publication_failure_release_the_pin_and_admission() {
    let f = Fixture::new();
    for cancel in [false, true] {
        for _ in 0..36 {
            f.ns.cancel.store(false, Ordering::Release);
            if cancel {
                f.ns.mode.store(7, Ordering::Release);
            } else {
                crate::runtime::fail_next_publish::<NativeFilesystem<Adapter>>(f.instance);
            }
            let (status, buffer) = invoke(
                &f.api,
                f.instance,
                Request::PrepareRename {
                    mount: f.mount,
                    node: f.node,
                },
                cancellation(&f),
            );
            if cancel {
                assert_eq!(status, OK);
                code(read(buffer), ErrorCode::Cancelled);
                assert_eq!(release(&f.api, f.instance, buffer.id), OK);
            } else {
                assert_eq!(status, RESOURCE_LIMIT);
                assert_eq!(buffer.id, 0);
            }
        }
    }
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 72);
    f.ns.cancel.store(false, Ordering::Release);
    f.ns.mode.store(0, Ordering::Release);
    assert_ne!(f.pin().handle, 0);
}
#[test]
fn adapter_cannot_report_sdk_only_states_as_unchanged_or_execute_again() {
    let f = Fixture::new();
    let pin = f.pin();
    f.ns.mode.store(2, Ordering::Release);
    let result = state(f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace));
    assert!(matches!(
        result.outcome,
        RenameOutcome::RecoveryRequired {
            source_location: RenameSourceLocation::Unknown,
            ..
        }
    ));
    code(
        f.attempt(pin.handle, 2, b"renamed", RenamePolicy::NoReplace),
        ErrorCode::StaleReference,
    );
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 1);
}
#[test]
fn exact_recovery_locator_and_actual_source_location_survive_query_and_replay() {
    let f = Fixture::new();
    let pin = f.pin();
    f.ns.mode.store(4, Ordering::Release);
    let result = state(f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace));
    let RenameOutcome::RecoveryRequired {
        source_location,
        recovery,
        ..
    } = &result.outcome
    else {
        panic!("recovery receipt")
    };
    assert_eq!(*source_location, RenameSourceLocation::Recovery);
    assert_eq!(recovery[0].address.bytes, b"/fixture/.stage/source-\xff");
    assert_eq!(
        f.call(Request::GetRenameState {
            prepared: pin.handle
        }),
        Response::RenameState(result.clone())
    );
    assert_eq!(
        f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace),
        Response::RenameState(result)
    );
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
}

#[test]
fn encode_and_publish_loss_with_late_cancel_and_ordinary_quota_pressure_keep_actual_result() {
    for encode in [true, false] {
        let f = Fixture::with_config(Config {
            max_calls: 2,
            max_buffers: 3,
            retained_buffer_bytes: 3 * DEFAULT_REPLY_BYTES,
            ..Config::default()
        });
        let pin = f.pin();
        let (status, first) = invoke(
            &f.api,
            f.instance,
            Request::Volumes,
            Cancellation::default(),
        );
        assert_eq!(status, OK);
        f.ns.mode.store(5, Ordering::Release);
        if encode {
            crate::runtime::fail_next_encode::<NativeFilesystem<Adapter>>(f.instance);
        } else {
            crate::runtime::fail_next_publish::<NativeFilesystem<Adapter>>(f.instance);
        }
        let (status, missing) = invoke(
            &f.api,
            f.instance,
            Request::Rename {
                prepared: pin.handle,
                attempt: 1,
                name: text(b"renamed"),
                policy: RenamePolicy::NoReplace,
            },
            cancellation(&f),
        );
        assert_eq!(status, RESOURCE_LIMIT);
        assert_eq!(missing.id, 0);
        assert_eq!(f.ns.drops.load(Ordering::Acquire), 0);
        let (status, second) = invoke(
            &f.api,
            f.instance,
            Request::Volumes,
            Cancellation::default(),
        );
        assert_eq!(status, OK);
        assert_eq!(
            invoke(
                &f.api,
                f.instance,
                Request::Volumes,
                Cancellation::default()
            )
            .0,
            RESOURCE_LIMIT
        );
        let (status, query) = invoke(
            &f.api,
            f.instance,
            Request::GetRenameState {
                prepared: pin.handle,
            },
            cancellation(&f),
        );
        assert_eq!(status, OK);
        let result = read(query);
        assert!(matches!(
            state(result.clone()).outcome,
            RenameOutcome::Renamed { .. }
        ));
        for buffer in [first, second, query] {
            assert_eq!(release(&f.api, f.instance, buffer.id), OK);
        }
        assert_eq!(
            f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace),
            result
        );
        assert_eq!(f.ns.executes.load(Ordering::Acquire), 1);
        assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
        assert_eq!(
            f.call(Request::CloseRename {
                prepared: pin.handle
            }),
            Response::Unit
        );
        assert_eq!(f.ns.drops.load(Ordering::Acquire), 1);
    }
}
#[test]
fn prepare_encoding_failure_retires_undisclosed_pins_and_returns_admission() {
    let f = Fixture::new();
    for _ in 0..40 {
        crate::runtime::fail_next_encode::<NativeFilesystem<Adapter>>(f.instance);
        let (status, buffer) = invoke(
            &f.api,
            f.instance,
            Request::PrepareRename {
                mount: f.mount,
                node: f.node,
            },
            Cancellation::default(),
        );
        assert_eq!(status, RESOURCE_LIMIT);
        assert_eq!(buffer.id, 0);
    }
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 40);
    let pins: Vec<_> = (0..32).map(|_| f.pin()).collect();
    assert_eq!(pins.len(), 32);
}
#[test]
fn undersized_receipt_budget_refuses_before_execute_or_any_write() {
    let f = Fixture::with_config(Config {
        reply_bytes: 4096,
        ..Config::default()
    });
    let pin = f.pin();
    code(
        f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace),
        ErrorCode::ResourceLimit,
    );
    let ready = state(f.call(Request::GetRenameState {
        prepared: pin.handle,
    }));
    assert_eq!(ready.attempt, 0);
    assert_eq!(ready.outcome, RenameOutcome::Ready);
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 0);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
}

#[test]
fn replacement_uses_newest_distinct_target_consent_and_preserves_that_target_receipt() {
    let f = Fixture::new();
    let pin = f.pin();
    f.ns.mode.store(1, Ordering::Release);
    let RenameOutcome::Conflict(first) =
        state(f.attempt(pin.handle, 1, b"target", RenamePolicy::NoReplace)).outcome
    else {
        panic!("first conflict")
    };
    code(
        f.attempt(
            pin.handle,
            1,
            b"target",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first.token,
            },
        ),
        ErrorCode::Invalid,
    );
    f.ns.target.store(101, Ordering::Release);
    f.ns.target_revision.store(2, Ordering::Release);
    let RenameOutcome::Conflict(second) = state(f.attempt(
        pin.handle,
        2,
        b"target",
        RenamePolicy::RecoverableCurrentEntry {
            conflict: first.token,
        },
    ))
    .outcome
    else {
        panic!("changed target needs fresh consent")
    };
    assert_ne!(second.token, first.token);
    assert_eq!(second.target.identity, 101u64.to_le_bytes());
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
    code(
        f.attempt(
            pin.handle,
            3,
            b"target",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first.token,
            },
        ),
        ErrorCode::Invalid,
    );
    let policy = RenamePolicy::RecoverableCurrentEntry {
        conflict: second.token,
    };
    let result = f.attempt(pin.handle, 3, b"target", policy.clone());
    let RenameOutcome::Renamed { recovery, .. } = state(result.clone()).outcome else {
        panic!("replacement")
    };
    assert_eq!(recovery.len(), 1);
    assert_eq!(recovery[0].identity, 101u64.to_le_bytes());
    assert_eq!(
        lock(&f.ns.recovered_targets).as_slice(),
        &[101u64.to_le_bytes().to_vec()]
    );
    assert_eq!(f.attempt(pin.handle, 3, b"target", policy), result);
    code(
        f.attempt(
            pin.handle,
            3,
            b"target",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: first.token,
            },
        ),
        ErrorCode::Invalid,
    );
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 3);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
}
#[test]
fn unqualified_alias_and_foreign_conflict_consent_never_enter_replacement() {
    for unqualified in [0, 1, 2] {
        let f = Fixture::new();
        if unqualified == 0 {
            f.ns.can_replace.store(false, Ordering::Release);
        }
        if unqualified == 1 {
            f.ns.recoverable.store(false, Ordering::Release);
        }
        if unqualified == 2 {
            f.ns.target.store(1, Ordering::Release);
        }
        let pin = f.pin();
        f.ns.mode.store(1, Ordering::Release);
        let RenameOutcome::Conflict(conflict) =
            state(f.attempt(pin.handle, 1, b"target", RenamePolicy::NoReplace)).outcome
        else {
            panic!("unqualified conflict")
        };
        assert!(!conflict.can_replace);
        code(
            f.attempt(
                pin.handle,
                2,
                b"target",
                RenamePolicy::RecoverableCurrentEntry {
                    conflict: conflict.token,
                },
            ),
            ErrorCode::Invalid,
        );
        assert_eq!(f.ns.executes.load(Ordering::Acquire), 1);
        assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
    }
    let f = Fixture::new();
    let first = f.pin();
    let other = f.pin();
    f.ns.mode.store(1, Ordering::Release);
    let RenameOutcome::Conflict(a) =
        state(f.attempt(first.handle, 1, b"target", RenamePolicy::NoReplace)).outcome
    else {
        panic!("first")
    };
    let RenameOutcome::Conflict(b) =
        state(f.attempt(other.handle, 1, b"target", RenamePolicy::NoReplace)).outcome
    else {
        panic!("other")
    };
    assert_ne!(a.token, b.token);
    code(
        f.attempt(
            first.handle,
            2,
            b"target",
            RenamePolicy::RecoverableCurrentEntry { conflict: b.token },
        ),
        ErrorCode::Invalid,
    );
    assert_eq!(f.ns.executes.load(Ordering::Acquire), 2);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 0);
}

#[test]
fn opening_pins_and_blocked_native_disposal_keep_their_count_admission() {
    for opening in [true, false] {
        let f = Fixture::new();
        let pins: Vec<_> = (0..if opening { 31 } else { 32 })
            .map(|_| f.pin())
            .collect();
        let gate = Arc::new(Gate::default());
        if opening {
            *lock(&f.ns.prepare_gate) = Some(gate.clone());
        } else {
            *lock(&f.ns.drop_gate) = Some(gate.clone());
        }
        std::thread::scope(|scope| {
            let _release = ReleaseGate(gate.clone());
            let instance = f.instance;
            let mount = f.mount;
            let node = f.node;
            let handle = pins[0].handle;
            let running = scope.spawn(move || {
                let api = descriptor::<NativeFilesystem<Adapter>>();
                if opening {
                    call(&api, instance, Request::PrepareRename { mount, node })
                } else {
                    call(&api, instance, Request::CloseRename { prepared: handle })
                }
            });
            gate.wait_entered();
            code(f.prepare(), ErrorCode::ResourceLimit);
            assert_eq!(f.ns.prepares.load(Ordering::Acquire), 32);
            if !opening {
                assert_eq!(f.ns.drops.load(Ordering::Acquire), 0);
            }
            gate.unblock();
            let response = running.join().unwrap();
            if opening {
                assert!(matches!(response, Response::RenamePrepared(_)));
            } else {
                assert_eq!(response, Response::Unit);
            }
        });
        *lock(&f.ns.prepare_gate) = None;
        *lock(&f.ns.drop_gate) = None;
        if !opening {
            assert_eq!(f.ns.drops.load(Ordering::Acquire), 1);
            assert_ne!(f.pin().handle, pins[0].handle);
        }
    }
}
#[test]
fn saturated_ordinary_call_preserves_query_and_close_within_config_total() {
    let f = Fixture::with_config(Config {
        max_calls: 2,
        max_buffers: 3,
        retained_buffer_bytes: 3 * DEFAULT_REPLY_BYTES,
        ..Config::default()
    });
    let pin = f.pin();
    let gate = Arc::new(Gate::default());
    *lock(&f.ns.gate) = Some(gate.clone());
    std::thread::scope(|scope| {
        let _release = ReleaseGate(gate.clone());
        let instance = f.instance;
        let running = scope.spawn(move || {
            let api = descriptor::<NativeFilesystem<Adapter>>();
            call(
                &api,
                instance,
                Request::Rename {
                    prepared: pin.handle,
                    attempt: 1,
                    name: text(b"renamed"),
                    policy: RenamePolicy::NoReplace,
                },
            )
        });
        gate.wait_entered();
        assert_eq!(
            invoke(
                &f.api,
                f.instance,
                Request::Volumes,
                Cancellation::default()
            )
            .0,
            BUSY
        );
        f.ns.cancel.store(true, Ordering::Release);
        let (status, buffer) = invoke(
            &f.api,
            f.instance,
            Request::GetRenameState {
                prepared: pin.handle,
            },
            cancellation(&f),
        );
        assert_eq!(status, OK);
        assert_eq!(state(read(buffer)).outcome, RenameOutcome::Executing);
        assert_eq!(release(&f.api, f.instance, buffer.id), OK);
        let (status, buffer) = invoke(
            &f.api,
            f.instance,
            Request::CloseRename {
                prepared: pin.handle,
            },
            cancellation(&f),
        );
        assert_eq!(status, OK);
        code(read(buffer), ErrorCode::Busy);
        assert_eq!(release(&f.api, f.instance, buffer.id), OK);
        assert_eq!(f.ns.drops.load(Ordering::Acquire), 0);
        f.ns.cancel.store(false, Ordering::Release);
        gate.unblock();
        assert!(matches!(
            state(running.join().unwrap()).outcome,
            RenameOutcome::Renamed { .. }
        ));
    });
    assert_eq!(
        f.call(Request::CloseRename {
            prepared: pin.handle
        }),
        Response::Unit
    );
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 1);
}

#[test]
fn wrong_pinned_basename_is_rejected_and_owned_prepare_replies_require_publication() {
    let f = Fixture::new();
    f.ns.mode.store(9, Ordering::Release);
    code(f.prepare(), ErrorCode::SourceChanged);
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 1);
    f.ns.mode.store(0, Ordering::Release);
    let prepared = f.pin();
    assert!(Reply::stateless(Response::RenamePrepared(prepared.clone())).is_err());
    assert_eq!(
        f.call(Request::CloseRename {
            prepared: prepared.handle
        }),
        Response::Unit
    );
}
#[test]
fn prepare_does_not_publish_after_its_mount_epoch_changes_during_native_open() {
    let f = Fixture::new();
    let pin = f.pin();
    let gate = Arc::new(Gate::default());
    *lock(&f.ns.prepare_gate) = Some(gate.clone());
    std::thread::scope(|scope| {
        let _release = ReleaseGate(gate.clone());
        let instance = f.instance;
        let mount = f.mount;
        let node = f.node;
        let running = scope.spawn(move || {
            let api = descriptor::<NativeFilesystem<Adapter>>();
            call(&api, instance, Request::PrepareRename { mount, node })
        });
        gate.wait_entered();
        assert!(matches!(
            state(f.attempt(pin.handle, 1, b"renamed", RenamePolicy::NoReplace)).outcome,
            RenameOutcome::Renamed { .. }
        ));
        gate.unblock();
        code(running.join().unwrap(), ErrorCode::StaleReference);
    });
    *lock(&f.ns.prepare_gate) = None;
    assert_eq!(f.ns.prepares.load(Ordering::Acquire), 2);
    assert_eq!(f.ns.drops.load(Ordering::Acquire), 1);
    assert_eq!(f.ns.writes.load(Ordering::Acquire), 1);
}
