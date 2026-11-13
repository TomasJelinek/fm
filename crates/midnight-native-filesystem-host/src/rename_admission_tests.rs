//! Observable Instance admission tests; the fake C boundary owns every reply until release.
use super::super::*;
use midnight_filesystem_plugin_api::{
    Address, AddressEncoding, ErrorCode, Kind, Metadata, NameEncoding, NativeText, Node,
    PreparedRename, RenameObject, RenameOutcome, RenamePolicy, RenameState, decode_request,
    encode_response,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex, MutexGuard, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(5);
const GATE_WAIT: Duration = Duration::from_secs(30);
const QUIET: Duration = Duration::from_millis(120);
const PREPARED: u64 = 400;
type Ticket = usize;

#[derive(Clone, Copy)]
struct Gates {
    call: bool,
    release: bool,
}
impl Gates {
    const OPEN: Self = Self {
        call: true,
        release: true,
    };
    const CALLBACK: Self = Self {
        call: false,
        release: false,
    };
    const RELEASE: Self = Self {
        call: true,
        release: false,
    };
}

struct Plan {
    request: Request,
    response: Option<Box<[u8]>>,
    gates: Gates,
    attempted: bool,
    entered: bool,
    returned: bool,
    releasing: bool,
    released: bool,
    completed: bool,
    native_cancelled: Option<bool>,
}
struct Reply {
    ticket: Ticket,
    bytes: Box<[u8]>,
}
#[derive(Clone, Copy, Debug, Default)]
struct HighWater {
    slots: usize,
    callbacks: usize,
    buffers: usize,
    bytes: usize,
    commitment: usize,
    controls: usize,
}
#[derive(Clone, Debug)]
struct Snapshot {
    slots: usize,
    callbacks: usize,
    buffers: usize,
    bytes: usize,
    commitment: usize,
    controls: usize,
    released: Vec<Ticket>,
    faults: Vec<String>,
    high: HighWater,
}
#[derive(Default)]
struct State {
    plans: Vec<Plan>,
    replies: BTreeMap<u64, Reply>,
    shutdown: bool,
    faults: Vec<String>,
    high: HighWater,
}
impl State {
    fn snapshot(&self) -> Snapshot {
        let callbacks = self
            .plans
            .iter()
            .filter(|p| p.entered && !p.returned)
            .count();
        let bytes = self.replies.values().map(|reply| reply.bytes.len()).sum();
        Snapshot {
            slots: self
                .plans
                .iter()
                .filter(|p| p.entered && !p.released)
                .count(),
            callbacks,
            buffers: self.replies.len(),
            bytes,
            commitment: callbacks * midnight_filesystem_plugin_api::DEFAULT_REPLY_BYTES as usize
                + bytes,
            controls: self
                .plans
                .iter()
                .filter(|p| {
                    p.entered
                        && !p.released
                        && matches!(
                            p.request,
                            Request::GetRenameState { .. } | Request::CloseRename { .. }
                        )
                })
                .count(),
            released: self
                .plans
                .iter()
                .enumerate()
                .filter_map(|(ticket, p)| p.released.then_some(ticket))
                .collect(),
            faults: self.faults.clone(),
            high: self.high,
        }
    }
    fn record_high_water(&mut self) {
        let now = self.snapshot();
        self.high.slots = self.high.slots.max(now.slots);
        self.high.callbacks = self.high.callbacks.max(now.callbacks);
        self.high.buffers = self.high.buffers.max(now.buffers);
        self.high.bytes = self.high.bytes.max(now.bytes);
        self.high.commitment = self.high.commitment.max(now.commitment);
        self.high.controls = self.high.controls.max(now.controls);
    }
}

#[derive(Default)]
struct Native {
    state: Mutex<State>,
    changed: Condvar,
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
impl Native {
    fn add(&self, request: Request, response: Response, gates: Gates) -> Ticket {
        let bytes = encode_response(
            &response,
            midnight_filesystem_plugin_api::DEFAULT_REPLY_BYTES as usize,
        )
        .unwrap()
        .into_boxed_slice();
        let mut state = lock(&self.state);
        let ticket = state.plans.len();
        state.plans.push(Plan {
            request,
            response: Some(bytes),
            gates,
            attempted: false,
            entered: false,
            returned: false,
            releasing: false,
            released: false,
            completed: false,
            native_cancelled: None,
        });
        ticket
    }
    fn attempted(&self, ticket: Ticket) {
        lock(&self.state).plans[ticket].attempted = true;
        self.changed.notify_all();
    }
    fn completed(&self, ticket: Ticket) {
        lock(&self.state).plans[ticket].completed = true;
        self.changed.notify_all();
    }
    fn wait(&self, ticket: Ticket, predicate: impl Fn(&Plan) -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            if predicate(&state.plans[ticket]) {
                return true;
            }
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
    }
    fn wait_gate<'a>(
        &self,
        mut state: MutexGuard<'a, State>,
        ticket: Ticket,
        release: bool,
    ) -> MutexGuard<'a, State> {
        let deadline = Instant::now() + GATE_WAIT;
        while !state.shutdown
            && !(if release {
                state.plans[ticket].gates.release
            } else {
                state.plans[ticket].gates.call
            })
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.faults.push(format!(
                    "Fixture timed out in {} gate for ticket {ticket}",
                    if release { "release" } else { "callback" }
                ));
                break;
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        state
    }
    fn open_call(&self, ticket: Ticket) {
        lock(&self.state).plans[ticket].gates.call = true;
        self.changed.notify_all();
    }
    fn open_release(&self, ticket: Ticket) {
        lock(&self.state).plans[ticket].gates.release = true;
        self.changed.notify_all();
    }
    fn shutdown(&self) {
        lock(&self.state).shutdown = true;
        self.changed.notify_all();
    }
    fn snapshot(&self) -> Snapshot {
        lock(&self.state).snapshot()
    }
    fn cancelled_seen(&self, ticket: Ticket) -> Option<bool> {
        lock(&self.state).plans[ticket].native_cancelled
    }
    fn fault(&self, message: &str) {
        lock(&self.state).faults.push(message.to_owned());
        self.changed.notify_all();
    }
    fn invoke(
        &self,
        request: Request,
        native_cancelled: bool,
    ) -> Result<(u64, *const u8, usize), abi::Status> {
        let mut state = lock(&self.state);
        let Some(ticket) = state
            .plans
            .iter()
            .position(|p| !p.entered && !p.completed && p.request == request)
        else {
            state
                .faults
                .push(format!("Unplanned native request: {request:?}"));
            return Err(abi::INVALID);
        };
        state.plans[ticket].entered = true;
        state.plans[ticket].native_cancelled = Some(native_cancelled);
        state.record_high_water();
        self.changed.notify_all();
        state = self.wait_gate(state, ticket, false);
        let Some(bytes) = state.plans[ticket].response.take() else {
            state
                .faults
                .push(format!("Missing reply for ticket {ticket}"));
            return Err(abi::INVALID);
        };
        let pointer = bytes.as_ptr();
        let length = bytes.len();
        let id = ticket as u64 + 1;
        state.replies.insert(id, Reply { ticket, bytes });
        state.plans[ticket].returned = true;
        state.record_high_water();
        self.changed.notify_all();
        Ok((id, pointer, length))
    }
    fn release(&self, id: u64) -> abi::Status {
        let mut state = lock(&self.state);
        let Some(reply) = state.replies.get(&id) else {
            state
                .faults
                .push(format!("Unknown or repeated reply release: {id}"));
            return abi::UNKNOWN_HANDLE;
        };
        let ticket = reply.ticket;
        state.plans[ticket].releasing = true;
        self.changed.notify_all();
        state = self.wait_gate(state, ticket, true);
        state.replies.remove(&id);
        state.plans[ticket].released = true;
        self.changed.notify_all();
        abi::OK
    }
}

// SAFETY: Harness owns this Arc allocation until every worker and Instance are dropped.
// Each synchronous C callback borrows it only during that retained lifetime.
unsafe fn native<'a>(context: u64) -> &'a Native {
    unsafe { &*(context as *const Native) }
}
unsafe extern "C" fn call(
    context: u64,
    input: *const abi::Call,
    output: *mut abi::Buffer,
) -> abi::Status {
    // SAFETY: Instance retains Native, the exact input, and writable output for this call.
    let native = unsafe { native(context) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: synchronous Instance::call provides a valid exact Call pointer.
        let input = unsafe { &*input };
        if input.struct_size < size_of::<abi::Call>() as u32
            || input
                .payload
                .validate(midnight_filesystem_plugin_api::DEFAULT_REQUEST_BYTES)
                .is_err()
        {
            native.fault("Invalid native call payload");
            return abi::INVALID;
        }
        // SAFETY: the bounded non-null request allocation remains owned by Instance::call.
        let payload = if input.payload.len == 0 {
            &[]
        } else {
            // SAFETY: validate above checked this nonempty pointer and bounded length.
            unsafe { std::slice::from_raw_parts(input.payload.data, input.payload.len as usize) }
        };
        let request = match decode_request(
            input.operation,
            payload,
            midnight_filesystem_plugin_api::DEFAULT_REQUEST_BYTES as usize,
        ) {
            Ok(request) => request,
            Err(_) => {
                native.fault("Native call contained an invalid encoded request");
                return abi::INVALID;
            }
        };
        let native_cancelled = input.cancel.cancelled.is_some_and(|cancelled| {
            // SAFETY: host retains this cancellation context until the callback returns.
            unsafe { cancelled(input.cancel.context) != 0 }
        });
        match native.invoke(request, native_cancelled) {
            Ok((id, data, len)) => {
                // SAFETY: exact caller-owned output; Native retains Box bytes through release.
                unsafe {
                    output.write(abi::Buffer {
                        id,
                        bytes: abi::Slice {
                            data,
                            len: len as u64,
                        },
                    });
                }
                abi::OK
            }
            Err(status) => status,
        }
    }));
    match result {
        Ok(status) => status,
        Err(_) => {
            native.fault("Fake native call panicked");
            abi::INVALID
        }
    }
}
unsafe extern "C" fn release(context: u64, buffer: u64) -> abi::Status {
    // SAFETY: Harness retains Native until all calls, buffers and Instance have retired.
    let native = unsafe { native(context) };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| native.release(buffer)))
        .unwrap_or_else(|_| {
            native.fault("Fake release callback panicked");
            abi::INVALID
        })
}
unsafe extern "C" fn destroy(context: u64) -> abi::Status {
    // SAFETY: the Harness-owned Native allocation outlives the last Instance reference.
    let native = unsafe { native(context) };
    let state = lock(&native.state);
    if state.replies.is_empty() && !state.plans.iter().any(|p| p.entered && !p.returned) {
        abi::OK
    } else {
        abi::BUSY
    }
}

struct Worker {
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Response, Failure>>,
    expected: Response,
    join: Option<JoinHandle<()>>,
}
struct Harness {
    // Field order also keeps Native alive for Instance's final destroy callback.
    instance: Arc<Instance>,
    native: Arc<Native>,
    workers: Vec<Worker>,
}
impl Harness {
    fn new() -> Self {
        let native = Arc::new(Native::default());
        let instance = Arc::new(Instance {
            library: None,
            instance: Arc::as_ptr(&native) as u64,
            call,
            release,
            destroy,
            poisoned: AtomicBool::new(false),
            admission: crate::admission::Admission::default(),
        });
        Self {
            instance,
            native,
            workers: Vec::new(),
        }
    }
    fn start(&mut self, request: Request, response: Response, gates: Gates) -> Ticket {
        self.start_with_cancel(request, response, gates, false)
    }
    fn start_with_cancel(
        &mut self,
        request: Request,
        response: Response,
        gates: Gates,
        initially_cancelled: bool,
    ) -> Ticket {
        let ticket = self.native.add(request.clone(), response.clone(), gates);
        assert_eq!(ticket, self.workers.len());
        let cancel = Arc::new(AtomicBool::new(initially_cancelled));
        let worker_cancel = cancel.clone();
        let instance = self.instance.clone();
        let native = self.native.clone();
        let (result_send, result) = mpsc::sync_channel(1);
        let (started_send, started) = mpsc::sync_channel(1);
        let join = thread::spawn(move || {
            let _ = started_send.send(());
            let response = instance.call(&request, &|| {
                native.attempted(ticket);
                worker_cancel.load(Ordering::Acquire)
            });
            native.completed(ticket);
            let _ = result_send.send(response);
            // Native is explicitly retained through dropping this Instance clone.
            drop(instance);
            drop(native);
        });
        self.workers.push(Worker {
            cancel,
            result,
            expected: response,
            join: Some(join),
        });
        assert!(
            started.recv_timeout(WAIT).is_ok(),
            "Worker {ticket} did not start"
        );
        ticket
    }
    fn browse(&mut self, node: u64, gates: Gates) -> Ticket {
        self.start(
            Request::Stat { mount: 1, node },
            Response::Node(Node {
                id: node,
                address: Address {
                    encoding: AddressEncoding::Opaque,
                    bytes: vec![node as u8],
                },
                name_encoding: NameEncoding::Utf8,
                raw_name: b"entry".to_vec(),
                display: "entry".to_owned(),
                kind: Kind::File,
                metadata: Metadata {
                    size: Some(1),
                    ..Metadata::default()
                },
            }),
            gates,
        )
    }
    fn read(&mut self, reader: u64, gates: Gates) -> Ticket {
        self.start(
            Request::ReadAt {
                reader,
                offset: 0,
                length: midnight_filesystem_plugin_api::MAX_READ,
            },
            Response::Content {
                bytes: vec![reader as u8; midnight_filesystem_plugin_api::MAX_READ as usize],
                eof: true,
            },
            gates,
        )
    }
    fn prepare(&mut self, gates: Gates) -> Ticket {
        self.start(
            Request::PrepareRename { mount: 1, node: 2 },
            prepared_reply(),
            gates,
        )
    }
    fn activate(&mut self) {
        let ticket = self.prepare(Gates::OPEN);
        self.expect_reply(ticket);
    }
    fn query(&mut self, gates: Gates, outcome: RenameOutcome) -> Ticket {
        self.start_with_cancel(
            Request::GetRenameState { prepared: PREPARED },
            state_reply(outcome),
            gates,
            true,
        )
    }
    fn close_rename(&mut self, gates: Gates) -> Ticket {
        self.start_with_cancel(
            Request::CloseRename { prepared: PREPARED },
            Response::Unit,
            gates,
            true,
        )
    }
    fn entered(&self, ticket: Ticket) {
        assert!(
            self.native.wait(ticket, |p| p.entered, WAIT),
            "Ticket {ticket} did not reach the native callback: {:?}",
            self.native.snapshot()
        );
    }
    fn releasing(&self, ticket: Ticket) {
        assert!(
            self.native.wait(ticket, |p| p.releasing, WAIT),
            "Ticket {ticket} did not reach reply release: {:?}",
            self.native.snapshot()
        );
    }
    fn attempted(&self, ticket: Ticket) {
        assert!(
            self.native
                .wait(ticket, |p| p.attempted || p.entered || p.completed, WAIT),
            "Ticket {ticket} did not attempt Instance::call"
        );
    }
    fn blocked(&self, ticket: Ticket, reason: &str) {
        assert!(
            !self.native.wait(ticket, |p| p.entered, QUIET),
            "{reason}; ticket {ticket} reached native code: {:?}",
            self.native.snapshot()
        );
    }
    fn pending(&self, ticket: Ticket, reason: &str) {
        assert!(
            !self.native.wait(ticket, |p| p.completed, QUIET),
            "{reason}; ticket {ticket} returned before real release"
        );
    }
    fn cancel(&self, ticket: Ticket) {
        self.workers[ticket].cancel.store(true, Ordering::Release);
    }
    fn expect_reply(&self, ticket: Ticket) {
        let result = self.workers[ticket]
            .result
            .recv_timeout(WAIT)
            .unwrap_or_else(|error| panic!("No reply for ticket {ticket}: {error}"));
        assert_eq!(
            result,
            Ok(self.workers[ticket].expected.clone()),
            "Unexpected transport reply for ticket {ticket}"
        );
    }
    fn expect_cancelled(&self, ticket: Ticket) {
        let result = self.workers[ticket]
            .result
            .recv_timeout(WAIT)
            .unwrap_or_else(|error| panic!("Cancellation did not return ticket {ticket}: {error}"));
        assert!(
            matches!(
                result,
                Err(Failure {
                    code: ErrorCode::Cancelled,
                    ..
                })
            ),
            "Waiting ticket {ticket} must return Cancelled, got {result:?}"
        );
    }
    fn drain(&self, tickets: &[Ticket]) {
        for &ticket in tickets {
            self.native.open_call(ticket);
            self.native.open_release(ticket);
        }
        for &ticket in tickets {
            self.expect_reply(ticket);
        }
    }
    fn assert_bounds(&self) {
        let snapshot = self.native.snapshot();
        assert!(snapshot.faults.is_empty(), "Fixture faults: {snapshot:?}");
        assert_eq!(
            snapshot.slots, 0,
            "Native call/reply ownership remained: {snapshot:?}"
        );
        assert_eq!(
            snapshot.buffers, 0,
            "Native reply allocation remained: {snapshot:?}"
        );
        assert!(
            snapshot.high.slots <= 8,
            "More than eight live call/reply slots: {snapshot:?}"
        );
        assert!(
            snapshot.high.callbacks <= 8,
            "More than eight native callbacks: {snapshot:?}"
        );
        assert!(
            snapshot.high.buffers <= 64,
            "More than 64 retained buffers: {snapshot:?}"
        );
        assert!(
            snapshot.high.bytes <= 16 * 1024 * 1024,
            "Retained replies exceeded 16 MiB: {snapshot:?}"
        );
        assert!(
            snapshot.high.commitment <= 16 * 1024 * 1024,
            "Callbacks plus retained replies exceeded 16 MiB: {snapshot:?}"
        );
        assert!(
            snapshot.high.controls <= 1,
            "Rename controls overlapped: {snapshot:?}"
        );
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        // Always unblock native callback/release gates and cancel host waiters before joining,
        // including when a RED assertion unwinds. No assertion or unwrap can double-panic here.
        for worker in &self.workers {
            worker.cancel.store(true, Ordering::Release);
        }
        self.native.shutdown();
        for worker in &mut self.workers {
            if let Some(join) = worker.join.take() {
                let _ = join.join();
            }
        }
    }
}

fn name(value: &[u8]) -> NativeText {
    NativeText {
        encoding: NameEncoding::Utf8,
        bytes: value.to_vec(),
    }
}
fn prepared_reply() -> Response {
    Response::RenamePrepared(PreparedRename {
        handle: PREPARED,
        source: RenameObject {
            identity: vec![1],
            revision: vec![2],
            name: name(b"source"),
            kind: Kind::File,
            metadata: Metadata {
                size: Some(1),
                ..Metadata::default()
            },
        },
        parent_identity: vec![3],
        recoverable_replace: true,
    })
}
fn state_reply(outcome: RenameOutcome) -> Response {
    Response::RenameState(RenameState {
        prepared: PREPARED,
        attempt: if matches!(outcome, RenameOutcome::Ready) {
            0
        } else {
            1
        },
        outcome,
    })
}

#[test]
fn rename_foundation_browse_seven_plus_independent_legacy_close() {
    // Break caught: reserving Rename capacity eagerly reduces historical Browse concurrency.
    let mut host = Harness::new();
    let reads: Vec<_> = (1..=7)
        .map(|node| host.browse(node, Gates::CALLBACK))
        .collect();
    for &ticket in &reads {
        host.entered(ticket);
    }
    let eighth = host.browse(8, Gates::OPEN);
    host.attempted(eighth);
    host.blocked(
        eighth,
        "The eighth Browse call must preserve legacy cleanup capacity",
    );

    let close = host.start(
        Request::CloseMount { mount: 90 },
        Response::Unit,
        Gates::CALLBACK,
    );
    host.entered(close);
    let full = host.native.snapshot();
    assert_eq!((full.slots, full.callbacks, full.buffers), (8, 8, 0));
    host.native.open_call(close);
    host.native.open_release(close);
    host.expect_reply(close);
    host.blocked(
        eighth,
        "Returning legacy cleanup must not lend its lane to Browse",
    );
    host.cancel(eighth);
    host.expect_cancelled(eighth);
    host.drain(&reads);
    host.assert_bounds();
}

#[test]
fn rename_foundation_first_prepare_waits_for_two_actual_releases() {
    // Break caught: a first Prepare enters at legacy count six or treats callback return
    // as admission return while its reply-release callback still owns the buffer.
    let mut host = Harness::new();
    let mut reads = Vec::new();
    for reader in 1..=7 {
        reads.push(host.read(
            reader,
            if reader <= 2 {
                Gates::RELEASE
            } else {
                Gates::CALLBACK
            },
        ));
    }
    host.releasing(reads[0]);
    host.releasing(reads[1]);
    for &ticket in &reads[2..] {
        host.entered(ticket);
    }
    let prepare = host.prepare(Gates::OPEN);
    host.attempted(prepare);
    host.blocked(
        prepare,
        "First Prepare cannot enter seven legacy call/reply slots",
    );

    let close = host.start(
        Request::CloseRead { reader: 90 },
        Response::Unit,
        Gates::OPEN,
    );
    host.expect_reply(close);
    host.native.open_call(reads[2]);
    host.releasing(reads[2]);
    host.blocked(
        prepare,
        "Returning a callback without releasing its reply frees no slot",
    );
    assert_eq!(host.native.snapshot().slots, 7);

    host.native.open_release(reads[0]);
    host.expect_reply(reads[0]);
    host.blocked(
        prepare,
        "One real release leaves six general slots; Prepare needs its own sixth",
    );
    assert_eq!(host.native.snapshot().slots, 6);
    let before_waiter = host.native.snapshot();
    let transition_waiter = host.browse(91, Gates::OPEN);
    host.attempted(transition_waiter);
    host.blocked(
        transition_waiter,
        "Pending Prepare must immediately prevent ordinary work from refilling slot seven",
    );
    host.cancel(transition_waiter);
    host.expect_cancelled(transition_waiter);
    host.blocked(
        transition_waiter,
        "Canceled ordinary transition waiter must send no native request",
    );
    let after_waiter = host.native.snapshot();
    assert_eq!(
        (
            after_waiter.slots,
            after_waiter.callbacks,
            after_waiter.buffers
        ),
        (
            before_waiter.slots,
            before_waiter.callbacks,
            before_waiter.buffers
        ),
        "Canceling the ordinary waiter cannot return a held callback or reply slot"
    );
    assert_eq!(
        after_waiter.bytes, before_waiter.bytes,
        "Canceling the ordinary waiter cannot free retained reply bytes"
    );
    assert_eq!(
        after_waiter.released, before_waiter.released,
        "Canceling the ordinary waiter cannot release another ticket"
    );
    host.native.open_release(reads[1]);
    host.expect_reply(reads[1]);
    host.entered(prepare);
    host.expect_reply(prepare);
    host.drain(&reads[2..]);
    host.assert_bounds();
}

#[test]
fn rename_foundation_canceled_transition_does_not_own_or_return_admission() {
    // Break caught: canceling a transition returns somebody else's callback/buffer permit,
    // leaks a waiter permit, or allows seven general calls after the reservation activates.
    let mut host = Harness::new();
    let mut reads = vec![host.read(1, Gates::RELEASE)];
    reads.extend((2..=7).map(|reader| host.read(reader, Gates::CALLBACK)));
    host.releasing(reads[0]);
    for &ticket in &reads[1..] {
        host.entered(ticket);
    }
    let abandoned = host.prepare(Gates::OPEN);
    host.attempted(abandoned);
    host.blocked(
        abandoned,
        "Transition must wait behind real legacy ownership",
    );
    let before = host.native.snapshot();
    host.cancel(abandoned);
    host.expect_cancelled(abandoned);
    host.blocked(abandoned, "Canceled transition must send no native request");
    let after = host.native.snapshot();
    assert_eq!((after.slots, after.callbacks, after.buffers), (7, 6, 1));
    assert_eq!(
        after.bytes, before.bytes,
        "Cancellation cannot release native reply bytes"
    );
    assert_eq!(
        after.released, before.released,
        "Cancellation cannot release another ticket"
    );

    host.native.open_release(reads[0]);
    host.expect_reply(reads[0]);
    host.native.open_call(reads[1]);
    host.native.open_release(reads[1]);
    host.expect_reply(reads[1]);
    let prepare = host.prepare(Gates::OPEN);
    host.expect_reply(prepare);
    let sixth = host.read(8, Gates::CALLBACK);
    host.entered(sixth);
    let seventh = host.read(9, Gates::OPEN);
    host.attempted(seventh);
    host.blocked(
        seventh,
        "Canceled transition must leave exactly six general slots after activation",
    );
    host.cancel(seventh);
    host.expect_cancelled(seventh);
    host.blocked(
        abandoned,
        "An abandoned transition cannot revive when slots become available",
    );
    host.drain(&reads[2..]);
    host.drain(&[sixth]);
    host.assert_bounds();
}

#[test]
fn rename_foundation_active_partition_preserves_close_and_query_during_canceled_rename() {
    // Break caught: Rename shares a seventh general slot, canceled native work returns
    // admission early, or receipt query/legacy close cannot both enter beside six general calls.
    let mut host = Harness::new();
    host.activate();
    let rename = host.start(
        Request::Rename {
            prepared: PREPARED,
            attempt: 1,
            name: name(b"destination"),
            policy: RenamePolicy::NoReplace,
        },
        state_reply(RenameOutcome::Renamed {
            name: name(b"destination"),
            same_entry: false,
            recovery: Vec::new(),
            durability_confirmed: false,
        }),
        Gates::CALLBACK,
    );
    host.entered(rename);
    let reads: Vec<_> = (1..=5)
        .map(|reader| host.read(reader, Gates::CALLBACK))
        .collect();
    for &ticket in &reads {
        host.entered(ticket);
    }
    host.cancel(rename);
    host.pending(
        rename,
        "Canceling an admitted Rename cannot finish a held native call",
    );
    let seventh = host.read(6, Gates::OPEN);
    host.attempted(seventh);
    host.blocked(
        seventh,
        "A canceled native Rename still occupies one of six general slots",
    );
    let close = host.start(
        Request::CloseList { cursor: 90 },
        Response::Unit,
        Gates::CALLBACK,
    );
    host.entered(close);
    let query = host.query(Gates::CALLBACK, RenameOutcome::Executing);
    host.entered(query);
    assert_eq!(
        host.native.cancelled_seen(query),
        Some(false),
        "Query ignores user cancellation"
    );
    assert_eq!(host.native.snapshot().slots, 8);
    host.cancel(seventh);
    host.expect_cancelled(seventh);

    host.native.open_call(query);
    host.releasing(query);
    host.pending(
        query,
        "Query admission survives callback return through actual reply release",
    );
    host.native.open_release(query);
    host.expect_reply(query);
    host.native.open_call(close);
    host.native.open_release(close);
    host.expect_reply(close);
    host.native.open_call(rename);
    host.releasing(rename);
    host.pending(
        rename,
        "Canceled Rename still owns its reply-release callback",
    );
    host.native.open_release(rename);
    host.expect_reply(rename);
    host.drain(&reads);
    host.assert_bounds();
}

#[test]
fn rename_foundation_held_reply_release_retains_permit_and_bytes() {
    // Break caught: decoding, callback return, or cancellation frees a general permit
    // or backing bytes before the native reply-release callback actually returns.
    let mut host = Harness::new();
    host.activate();
    let reads: Vec<_> = (1..=6)
        .map(|reader| host.read(reader, Gates::RELEASE))
        .collect();
    for &ticket in &reads {
        host.releasing(ticket);
    }
    let retained = host.native.snapshot();
    assert_eq!(
        (retained.slots, retained.callbacks, retained.buffers),
        (6, 0, 6)
    );
    assert!(retained.bytes >= 6 * midnight_filesystem_plugin_api::MAX_READ as usize);
    let seventh = host.read(7, Gates::OPEN);
    host.attempted(seventh);
    host.blocked(
        seventh,
        "Six held reply releases exhaust the general partition",
    );
    host.cancel(reads[1]);
    host.pending(
        reads[1],
        "Cancellation cannot finish an admitted buffer release",
    );

    let close = host.start(
        Request::CloseRead { reader: 90 },
        Response::Unit,
        Gates::RELEASE,
    );
    host.releasing(close);
    let query = host.query(Gates::RELEASE, RenameOutcome::Ready);
    host.releasing(query);
    assert_eq!(host.native.cancelled_seen(query), Some(false));
    let full = host.native.snapshot();
    assert_eq!((full.slots, full.callbacks, full.buffers), (8, 0, 8));
    assert!(full.bytes > retained.bytes);
    assert_eq!(full.released, retained.released);
    host.native.open_release(close);
    host.expect_reply(close);
    host.native.open_release(query);
    host.expect_reply(query);
    host.blocked(
        seventh,
        "Reserved-lane releases cannot return an occupied general slot",
    );
    assert_eq!(host.native.snapshot().bytes, retained.bytes);

    host.native.open_release(reads[0]);
    host.expect_reply(reads[0]);
    host.entered(seventh);
    host.expect_reply(seventh);
    host.drain(&reads[1..]);
    host.assert_bounds();
}

#[test]
fn rename_foundation_query_and_close_share_one_control_until_reply_release() {
    // Break caught: query/close have separate control slots, are cancelable, or release
    // their shared lane before the actual reply-buffer release while aggregate work is full.
    let mut host = Harness::new();
    host.activate();
    let reads: Vec<_> = (1..=6)
        .map(|reader| host.read(reader, Gates::CALLBACK))
        .collect();
    for &ticket in &reads {
        host.entered(ticket);
    }
    let legacy = host.start(
        Request::CloseMount { mount: 90 },
        Response::Unit,
        Gates::RELEASE,
    );
    host.releasing(legacy);
    let query = host.query(Gates::CALLBACK, RenameOutcome::Ready);
    host.entered(query);
    assert_eq!(host.native.cancelled_seen(query), Some(false));
    let close = host.close_rename(Gates::RELEASE);
    host.blocked(
        close,
        "CloseRename must wait for the single Query/CloseRename control lane",
    );
    assert_eq!(host.native.snapshot().slots, 8);

    host.native.open_call(query);
    host.releasing(query);
    host.blocked(
        close,
        "Query keeps the control lane while native buffer release is held",
    );
    host.native.open_release(query);
    host.expect_reply(query);
    host.releasing(close);
    assert_eq!(
        host.native.cancelled_seen(close),
        Some(false),
        "CloseRename ignores user cancellation"
    );
    let next_query = host.start_with_cancel(
        Request::GetRenameState { prepared: PREPARED },
        Response::Failed(Failure::new(
            ErrorCode::StaleReference,
            "Prepared handle is closed",
        )),
        Gates::RELEASE,
        true,
    );
    host.blocked(
        next_query,
        "Query must wait for CloseRename's retained reply in the same lane",
    );
    assert_eq!(host.native.snapshot().slots, 8);
    host.native.open_release(close);
    host.expect_reply(close);
    host.releasing(next_query);
    assert_eq!(host.native.cancelled_seen(next_query), Some(false));
    host.native.open_release(next_query);
    host.expect_reply(next_query);
    host.native.open_release(legacy);
    host.expect_reply(legacy);
    host.drain(&reads);
    host.assert_bounds();
}

fn first_control_waits_for_one_actual_release(request: Request, response: Response) {
    // Break caught: a first control starts beside seven general owners, waits for
    // two releases like Prepare, or stops reserving its lane after its first reply.
    let mut host = Harness::new();
    let mut reads = vec![host.read(1, Gates::RELEASE)];
    reads.extend((2..=7).map(|reader| host.read(reader, Gates::CALLBACK)));
    host.releasing(reads[0]);
    for &ticket in &reads[1..] {
        host.entered(ticket);
    }
    let control = host.start_with_cancel(request, response, Gates::CALLBACK, true);
    host.blocked(
        control,
        "First control must drain one of seven general owners",
    );
    host.pending(control, "User cancellation cannot end a waiting control");

    let legacy = host.start(
        Request::CloseRead { reader: 90 },
        Response::Unit,
        Gates::RELEASE,
    );
    host.releasing(legacy);
    assert_eq!(host.native.snapshot().slots, 8);
    host.native.open_call(reads[1]);
    host.releasing(reads[1]);
    host.blocked(
        control,
        "Callback return alone leaves the general owner live",
    );

    host.native.open_release(reads[0]);
    host.expect_reply(reads[0]);
    host.entered(control);
    assert_eq!(host.native.cancelled_seen(control), Some(false));
    assert_eq!(host.native.snapshot().slots, 8);
    let seventh = host.read(8, Gates::OPEN);
    host.attempted(seventh);
    host.blocked(
        seventh,
        "First control must prevent general slot seven from refilling",
    );

    host.native.open_call(control);
    host.releasing(control);
    host.pending(
        control,
        "The control owns its lane through native reply release",
    );
    host.native.open_release(legacy);
    host.expect_reply(legacy);
    host.blocked(seventh, "Legacy cleanup cannot return a general permit");
    host.native.open_release(control);
    host.expect_reply(control);
    host.blocked(
        seventh,
        "The six-call general limit persists after first control closes",
    );
    host.cancel(seventh);
    host.expect_cancelled(seventh);
    host.drain(&reads[1..]);
    host.assert_bounds();
}

#[test]
fn rename_foundation_first_query_waits_for_one_actual_release() {
    first_control_waits_for_one_actual_release(
        Request::GetRenameState { prepared: PREPARED },
        state_reply(RenameOutcome::Ready),
    );
}

#[test]
fn rename_foundation_first_close_waits_for_one_actual_release() {
    first_control_waits_for_one_actual_release(
        Request::CloseRename { prepared: PREPARED },
        Response::Unit,
    );
}
