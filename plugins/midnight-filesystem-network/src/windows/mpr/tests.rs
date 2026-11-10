use super::*;
use std::{
    mem::{offset_of, size_of},
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

enum Step {
    Page(Vec<Resource>),
    More(u32),
    Error(u32),
    Count(u32),
    Block(Arc<Gate>),
    BlockError(Arc<Gate>, u32),
    End,
}
#[derive(Default)]
struct Gate {
    entered: (Mutex<bool>, Condvar),
    released: (Mutex<bool>, Condvar),
}
impl Gate {
    fn enter(&self) {
        *self.entered.0.lock().unwrap() = true;
        self.entered.1.notify_all();
        let result = self
            .released
            .1
            .wait_timeout_while(
                self.released.0.lock().unwrap(),
                Duration::from_secs(3),
                |value| !*value,
            )
            .unwrap();
        assert!(!result.1.timed_out());
    }
    fn wait(&self) {
        let result = self
            .entered
            .1
            .wait_timeout_while(
                self.entered.0.lock().unwrap(),
                Duration::from_secs(3),
                |value| !*value,
            )
            .unwrap();
        assert!(!result.1.timed_out());
    }
    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}
#[derive(Default)]
struct Fake {
    steps: Mutex<VecDeque<Step>>,
    opens: Mutex<Vec<(u32, Option<Resource>)>>,
    sizes: Mutex<Vec<usize>>,
    closed: AtomicUsize,
    calls: AtomicUsize,
    open_error: u32,
    open_scope_error: Option<(u32, u32)>,
    open_gate: Option<Arc<Gate>>,
    cancel_on_open: Option<Arc<AtomicBool>>,
    extended: Option<u32>,
}
impl Fake {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            ..Default::default()
        })
    }
}
impl Api for Fake {
    fn open(&self, scope: u32, parent: Option<&Resource>) -> (u32, usize) {
        self.opens.lock().unwrap().push((scope, parent.cloned()));
        if let Some(gate) = &self.open_gate {
            gate.enter();
        }
        if let Some(cancel) = &self.cancel_on_open {
            cancel.store(true, Ordering::SeqCst);
        }
        (
            self.open_scope_error
                .filter(|(failed_scope, _)| *failed_scope == scope)
                .map_or(self.open_error, |(_, error)| error),
            42,
        )
    }
    fn next(&self, handle: usize, count: &mut u32, buffer: &mut Buffer, bytes: &mut u32) -> u32 {
        assert_eq!(handle, 42);
        assert_eq!(*count, 128);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.sizes.lock().unwrap().push(buffer.len());
        match self.steps.lock().unwrap().pop_front().unwrap_or(Step::End) {
            Step::Page(records) => {
                write_page(buffer, &records);
                *count = records.len() as u32;
                NO_ERROR
            }
            Step::More(size) => {
                *bytes = size;
                *count = u32::MAX;
                buffer.bytes_mut().fill(0xff);
                ERROR_MORE_DATA
            }
            Step::Error(code) => code,
            Step::Count(value) => {
                *count = value;
                NO_ERROR
            }
            Step::Block(gate) => {
                gate.enter();
                write_page(buffer, &[server()]);
                *count = 1;
                NO_ERROR
            }
            Step::BlockError(gate, error) => {
                gate.enter();
                error
            }
            Step::End => {
                *count = u32::MAX;
                buffer.bytes_mut().fill(0xff);
                ERROR_NO_MORE_ITEMS
            }
        }
    }
    fn close(&self, handle: usize) {
        assert_eq!(handle, 42);
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
    fn extended_error(&self) -> Option<u32> {
        self.extended
    }
}
fn context() -> Context<'static> {
    Context {
        cancelled: &|| false,
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
fn server() -> Resource {
    Resource {
        scope: RESOURCE_CONTEXT,
        kind: RESOURCETYPE_DISK,
        display_type: resource::DISPLAY_SERVER,
        usage: RESOURCEUSAGE_CONTAINER,
        remote: Some(wide(r"\\東京")),
        provider: Some(wide("Provider A")),
        ..Default::default()
    }
}
fn write_page(buffer: &mut Buffer, records: &[Resource]) {
    buffer.bytes_mut().fill(0);
    let base = buffer.pointer() as usize;
    let mut position = records.len() * size_of::<NETRESOURCEW>();
    for (index, resource) in records.iter().enumerate() {
        let start = index * size_of::<NETRESOURCEW>();
        for (offset, value) in [
            (offset_of!(NETRESOURCEW, dwScope), resource.scope),
            (offset_of!(NETRESOURCEW, dwType), resource.kind),
            (
                offset_of!(NETRESOURCEW, dwDisplayType),
                resource.display_type,
            ),
            (offset_of!(NETRESOURCEW, dwUsage), resource.usage),
        ] {
            buffer.bytes_mut()[start + offset..start + offset + 4]
                .copy_from_slice(&value.to_ne_bytes());
        }
        for (offset, field) in [
            (offset_of!(NETRESOURCEW, lpLocalName), &resource.local),
            (offset_of!(NETRESOURCEW, lpRemoteName), &resource.remote),
            (offset_of!(NETRESOURCEW, lpComment), &resource.comment),
            (offset_of!(NETRESOURCEW, lpProvider), &resource.provider),
        ] {
            if let Some(units) = field {
                buffer.bytes_mut()[start + offset..start + offset + size_of::<usize>()]
                    .copy_from_slice(&(base + position).to_ne_bytes());
                for unit in units.iter().chain(Some(&0)) {
                    buffer.bytes_mut()[position..position + 2].copy_from_slice(&unit.to_ne_bytes());
                    position += 2;
                }
            }
        }
    }
}

#[test]
fn root_and_selected_resource_use_exact_scopes_and_single_nonrecursive_cursors() {
    let mut second = server();
    second.provider = Some(wide("Provider B"));
    let api = Fake::new(vec![Step::Page(vec![server(), second]), Step::End]);
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    let first = cursor.next(&context()).unwrap().unwrap();
    let second = cursor.next(&context()).unwrap().unwrap();
    assert_eq!(first.display, second.display);
    assert_ne!(first.target, second.target);
    assert!(cursor.next(&context()).unwrap().is_none());
    drop(cursor);
    assert_eq!(*api.opens.lock().unwrap(), vec![(RESOURCE_GLOBALNET, None)]);
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
    let value = Resource::decode(&first.target.key).unwrap();
    let cursor = begin_with(&first.target, &context(), api.clone()).unwrap();
    drop(cursor);
    assert_eq!(
        api.opens.lock().unwrap()[1],
        (RESOURCE_GLOBALNET, Some(value))
    );
    let share = Windows::manual(r"\\server\share").unwrap();
    assert!(begin_with(&share.target, &context(), api.clone()).is_err());
    assert_eq!(api.opens.lock().unwrap().len(), 2);
}

#[test]
fn root_lists_provider_containers_without_opening_the_slow_network_context() {
    let providers: Vec<_> = ["Provider A", "Provider B"]
        .into_iter()
        .map(|provider| Resource {
            scope: RESOURCE_GLOBALNET,
            kind: RESOURCETYPE_ANY,
            display_type: RESOURCEDISPLAYTYPE_NETWORK,
            usage: RESOURCEUSAGE_CONTAINER | 0x8000_0000,
            remote: Some(wide("Network")),
            provider: Some(wide(provider)),
            ..Default::default()
        })
        .collect();
    let api = Arc::new(Fake {
        steps: Mutex::new(vec![Step::Page(providers.clone()), Step::End].into()),
        // RESOURCE_CONTEXT can spend its whole deadline looking for browser
        // services before returning only an unnamed Entire Network root.
        open_scope_error: Some((RESOURCE_CONTEXT, ERROR_TIMEOUT)),
        ..Default::default()
    });
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    let mut records = Vec::new();
    while let Some(record) = cursor.next(&context()).unwrap() {
        records.push(record);
    }
    assert_eq!(records.len(), providers.len());
    assert_eq!(*api.opens.lock().unwrap(), vec![(RESOURCE_GLOBALNET, None)]);
    for (record, expected) in records.iter().zip(&providers) {
        assert_eq!(record.target.kind, NodeKind::Container);
        assert_eq!(Resource::decode(&record.target.key).unwrap(), *expected);
    }
    assert_eq!(records[0].display, records[1].display);
    assert_ne!(records[0].target, records[1].target);
    let child = begin_with(&records[1].target, &context(), api.clone()).unwrap();
    assert_eq!(
        api.opens.lock().unwrap()[1],
        (RESOURCE_GLOBALNET, Some(providers[1].clone()))
    );
    drop(child);
    assert_eq!(api.closed.load(Ordering::SeqCst), 2);
}

fn late_native_failure(open: bool, cancelled: bool) -> (Failure, usize) {
    let gate = Arc::new(Gate::default());
    let api = Arc::new(Fake {
        steps: Mutex::new(if open {
            VecDeque::new()
        } else {
            vec![Step::BlockError(gate.clone(), ERROR_EXTENDED_ERROR)].into()
        }),
        open_error: if open { ERROR_EXTENDED_ERROR } else { NO_ERROR },
        open_gate: open.then(|| gate.clone()),
        extended: Some(6118),
        ..Default::default()
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker_api = api.clone();
    let worker = std::thread::spawn(move || {
        let cancelled = || worker_cancel.load(Ordering::SeqCst);
        let ctx = Context {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        begin_with(&Windows::root().target, &ctx, worker_api)
            .and_then(|mut cursor| cursor.next(&ctx))
    });
    gate.wait();
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(api.closed.load(Ordering::SeqCst), 0);
    cancel.store(cancelled, Ordering::SeqCst);
    gate.release();
    (
        worker.join().unwrap().unwrap_err(),
        api.closed.load(Ordering::SeqCst),
    )
}

#[test]
fn late_native_open_failure_retains_the_provider_diagnostic() {
    let (error, closed) = late_native_failure(true, false);
    assert_eq!(error.code, ErrorCode::Io);
    assert_eq!(error.os_code, Some(ERROR_EXTENDED_ERROR as i32));
    assert!(error.message.contains("6118"));
    assert_eq!(closed, 0, "failed open never grants handle ownership");
}

#[test]
fn late_native_enumeration_failure_retains_the_provider_diagnostic() {
    let (error, closed) = late_native_failure(false, false);
    assert_eq!(error.code, ErrorCode::Io);
    assert_eq!(error.os_code, Some(ERROR_EXTENDED_ERROR as i32));
    assert!(error.message.contains("6118"));
    assert_eq!(closed, 1);
}

#[test]
fn cancellation_still_wins_over_a_late_native_provider_failure() {
    for open in [true, false] {
        let (error, closed) = late_native_failure(open, true);
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert_eq!(closed, usize::from(!open));
    }
}
#[test]
fn more_data_uses_bounded_growth_and_ignores_undefined_error_buffer_and_count() {
    let api = Fake::new(vec![
        Step::More(20_001),
        Step::Page(vec![server()]),
        Step::End,
    ]);
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    assert!(cursor.next(&context()).unwrap().is_some());
    assert!(cursor.next(&context()).unwrap().is_none());
    let sizes = api.sizes.lock().unwrap();
    assert_eq!(sizes[0], 16 * 1024);
    assert!(sizes[1] >= 20_001 && sizes[1] <= buffer::MAX_BUFFER);
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
    let api = Fake::new(vec![Step::More(buffer::MAX_BUFFER as u32 + 1)]);
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    assert_eq!(
        cursor.next(&context()).unwrap_err().code,
        ErrorCode::ResourceLimit
    );
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
}
#[test]
fn invalid_count_and_provider_errors_close_once_without_reissuing_a_failed_cursor() {
    for step in [
        Step::Count(0),
        Step::Count(129),
        Step::Error(ERROR_ACCESS_DENIED),
    ] {
        let api = Fake::new(vec![step]);
        let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
        assert!(cursor.next(&context()).is_err());
        assert!(cursor.next(&context()).is_err());
        drop(cursor);
        assert_eq!(api.calls.load(Ordering::SeqCst), 1);
        assert_eq!(api.closed.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn native_error_categories_preserve_unavailable_offline_denied_authentication_and_timeout() {
    for (code, expected) in [
        (ERROR_BAD_PROVIDER, ErrorCode::Unavailable),
        (ERROR_BAD_NETPATH, ErrorCode::Offline),
        (ERROR_ACCESS_DENIED, ErrorCode::PermissionDenied),
        (ERROR_LOGON_FAILURE, ErrorCode::AuthenticationRequired),
        (ERROR_NOT_AUTHENTICATED, ErrorCode::AuthenticationRequired),
        (
            ERROR_SESSION_CREDENTIAL_CONFLICT,
            ErrorCode::AuthenticationRequired,
        ),
        (ERROR_TIMEOUT, ErrorCode::TimedOut),
        (ERROR_CANCELLED, ErrorCode::Cancelled),
    ] {
        let result = failure(code, false);
        assert_eq!(result.code, expected);
        assert_eq!(result.os_code, Some(code as i32));
    }
    assert_eq!(failure(ERROR_NO_NETWORK, true).code, ErrorCode::Unavailable);
    assert_eq!(failure(ERROR_NO_NETWORK, false).code, ErrorCode::Offline);
}
#[test]
fn cancellation_around_open_closes_only_a_successfully_acquired_handle() {
    let flag = Arc::new(AtomicBool::new(true));
    let cancel = || flag.load(Ordering::SeqCst);
    let ctx = Context {
        cancelled: &cancel,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    let api = Fake::new(vec![]);
    assert_eq!(
        begin_with(&Windows::root().target, &ctx, api.clone())
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(api.opens.lock().unwrap().is_empty());
    flag.store(false, Ordering::SeqCst);
    let api = Arc::new(Fake {
        cancel_on_open: Some(flag.clone()),
        ..Default::default()
    });
    assert_eq!(
        begin_with(&Windows::root().target, &ctx, api.clone())
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
    let api = Arc::new(Fake {
        open_error: ERROR_ACCESS_DENIED,
        ..Default::default()
    });
    assert!(begin_with(&Windows::root().target, &context(), api.clone()).is_err());
    assert_eq!(
        api.closed.load(Ordering::SeqCst),
        0,
        "failed calls do not grant ownership even if output is non-null"
    );
}
#[test]
fn blocked_native_call_keeps_cursor_ownership_until_actual_return_then_observes_cancellation() {
    let gate = Arc::new(Gate::default());
    let api = Fake::new(vec![Step::Block(gate.clone())]);
    let cancel = Arc::new(AtomicBool::new(false));
    let thread_api = api.clone();
    let thread_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        let check = || thread_cancel.load(Ordering::SeqCst);
        let ctx = Context {
            cancelled: &check,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let mut cursor = begin_with(&Windows::root().target, &ctx, thread_api).unwrap();
        cursor.next(&ctx)
    });
    gate.wait();
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(api.closed.load(Ordering::SeqCst), 0);
    assert_eq!(api.calls.load(Ordering::SeqCst), 1);
    gate.release();
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
}

#[test]
fn extended_provider_code_is_opaque_even_when_its_number_matches_a_win32_error() {
    let api = Arc::new(Fake {
        steps: Mutex::new(VecDeque::from([Step::Error(ERROR_EXTENDED_ERROR)])),
        extended: Some(ERROR_LOGON_FAILURE),
        ..Default::default()
    });
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    let error = cursor.next(&context()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert_eq!(error.os_code, Some(ERROR_EXTENDED_ERROR as i32));
    assert!(error.message.contains(&ERROR_LOGON_FAILURE.to_string()));
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
}

#[test]
fn a_success_count_without_fresh_native_bytes_cannot_replay_the_previous_page() {
    let api = Fake::new(vec![Step::Page(vec![server()]), Step::Count(1)]);
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    assert!(cursor.next(&context()).unwrap().is_some());
    assert!(cursor.next(&context()).is_err());
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
}

#[test]
fn blocked_return_after_deadline_is_timed_out_and_releases_only_after_actual_return() {
    let gate = Arc::new(Gate::default());
    let api = Fake::new(vec![Step::Block(gate.clone())]);
    let thread_api = api.clone();
    let worker = std::thread::spawn(move || {
        let ctx = Context {
            cancelled: &|| false,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let mut cursor = begin_with(&Windows::root().target, &ctx, thread_api).unwrap();
        cursor.next(&ctx)
    });
    gate.wait();
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(api.closed.load(Ordering::SeqCst), 0);
    gate.release();
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        ErrorCode::TimedOut
    );
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
}

#[test]
fn discovery_filters_print_and_hidden_resources_but_manual_hidden_share_is_explicit() {
    let mut printer = server();
    printer.kind = RESOURCETYPE_PRINT;
    let mut hidden = server();
    hidden.display_type = resource::DISPLAY_SHARE;
    hidden.usage = RESOURCEUSAGE_CONNECTABLE;
    hidden.remote = Some(wide(r"\\host\C$"));
    let mut admin = hidden.clone();
    admin.display_type = RESOURCEDISPLAYTYPE_SHAREADMIN;
    let api = Fake::new(vec![
        Step::Page(vec![printer, hidden, admin]),
        Step::Page(vec![server()]),
        Step::End,
    ]);
    let mut cursor = begin_with(&Windows::root().target, &context(), api).unwrap();
    assert_eq!(
        cursor.next(&context()).unwrap().unwrap().target.kind,
        NodeKind::Server
    );
    assert!(cursor.next(&context()).unwrap().is_none());
    assert!(matches!(
        Windows::activate(&Windows::manual(r"\\host\C$").unwrap().target, &context()).unwrap(),
        ActivationTarget::Native(_)
    ));
}

#[test]
fn nonprogressing_growth_and_filtered_streams_hit_budgets_instead_of_looping_forever() {
    let api = Fake::new((0..20).map(|_| Step::More(0)).collect());
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    assert_eq!(
        cursor.next(&context()).unwrap_err().code,
        ErrorCode::ResourceLimit
    );
    assert!(api.calls.load(Ordering::SeqCst) <= 8);
    assert_eq!(api.closed.load(Ordering::SeqCst), 1);
    let mut printer = server();
    printer.kind = RESOURCETYPE_PRINT;
    let api = Fake::new(
        (0..129)
            .map(|_| Step::Page(vec![printer.clone()]))
            .collect(),
    );
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    assert_eq!(
        cursor.next(&context()).unwrap_err().code,
        ErrorCode::ResourceLimit
    );
    assert_eq!(api.calls.load(Ordering::SeqCst), 128);
    let api = Fake::new(
        (0..33)
            .map(|_| Step::Page(vec![printer.clone(); 128]))
            .collect(),
    );
    let mut cursor = begin_with(&Windows::root().target, &context(), api.clone()).unwrap();
    assert_eq!(
        cursor.next(&context()).unwrap_err().code,
        ErrorCode::ResourceLimit
    );
    assert_eq!(api.calls.load(Ordering::SeqCst), 33);
}
