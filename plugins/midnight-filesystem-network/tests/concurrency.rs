#[path = "network_support/mod.rs"]
mod support;
use midnight_filesystem_network::{adapter::*, api::*};
use std::{
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
static SERIAL: Mutex<()> = Mutex::new(());
static BEGINS: AtomicUsize = AtomicUsize::new(0);
#[derive(Default)]
struct Control {
    block_at: Option<usize>,
    entered: bool,
    release: bool,
    block_drop: bool,
    drop_entered: bool,
    release_drop: bool,
    cancel_observed: bool,
}
struct Gate {
    control: Mutex<Control>,
    changed: Condvar,
}
fn gate() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(|| Gate {
        control: Mutex::default(),
        changed: Condvar::new(),
    })
}
struct Release;
impl Drop for Release {
    fn drop(&mut self) {
        let mut control = gate().control.lock().unwrap_or_else(|e| e.into_inner());
        control.release = true;
        control.release_drop = true;
        gate().changed.notify_all();
    }
}
fn reset() {
    *gate().control.lock().unwrap_or_else(|e| e.into_inner()) = Control::default();
    BEGINS.store(0, Ordering::SeqCst);
}
fn wait_for(predicate: impl Fn(&Control) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut c = gate().control.lock().unwrap_or_else(|e| e.into_inner());
    while !predicate(&c) {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("fixture did not reach expected state");
        c = gate().changed.wait_timeout(c, remaining).unwrap().0;
    }
}
fn release() {
    gate().control.lock().unwrap().release = true;
    gate().changed.notify_all();
}
struct Fixture;
struct Cursor {
    index: usize,
    generation: usize,
}
impl Drop for Cursor {
    fn drop(&mut self) {
        let mut c = gate().control.lock().unwrap_or_else(|e| e.into_inner());
        if c.block_drop {
            c.drop_entered = true;
            gate().changed.notify_all();
            while !c.release_drop {
                c = gate().changed.wait(c).unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}
impl Adapter for Fixture {
    type Cursor = Cursor;
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"root".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(_: &Target) -> Result<(), Failure> {
        Ok(())
    }
    fn manual(input: &str) -> Result<Record, Failure> {
        Ok(Record {
            target: Target {
                kind: NodeKind::Server,
                key: input.as_bytes().to_vec(),
            },
            display: input.into(),
        })
    }
    fn begin(_: &Target, _: &Context<'_>) -> Result<Cursor, Failure> {
        Ok(Cursor {
            index: 0,
            generation: BEGINS.fetch_add(1, Ordering::SeqCst) + 1,
        })
    }
    fn next(cursor: &mut Cursor, context: &Context<'_>) -> Result<Option<Record>, Failure> {
        let mut c = gate().control.lock().unwrap_or_else(|e| e.into_inner());
        if c.block_at == Some(cursor.index) {
            c.entered = true;
            gate().changed.notify_all();
            while !c.release {
                if (context.cancelled)() {
                    c.cancel_observed = true;
                    gate().changed.notify_all();
                }
                c = gate()
                    .changed
                    .wait_timeout(c, Duration::from_millis(5))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
        drop(c);
        if cursor.index == 2 {
            return Ok(None);
        }
        let index = cursor.index;
        cursor.index += 1;
        Ok(Some(Record {
            target: Target {
                kind: NodeKind::Server,
                key: format!("generation-{}-{index}", cursor.generation).into_bytes(),
            },
            display: format!("Generation {}", cursor.generation),
        }))
    }
    fn activate(target: &Target, context: &Context<'_>) -> Result<ActivationTarget, Failure> {
        context.check()?;
        Ok(ActivationTarget::WithinProvider(Address {
            encoding: AddressEncoding::Opaque,
            bytes: target.key.clone(),
        }))
    }
}
fn begin(client: &support::Client, mount: u64, node: u64) -> u64 {
    let Response::Page { cursor, .. } = client.call(Request::ListBegin {
        mount,
        directory: node,
        max_entries: 1,
    }) else {
        panic!("missing page")
    };
    cursor
}
fn busy(response: Response) {
    assert!(
        matches!(
            response,
            Response::Failed(Failure {
                code: ErrorCode::Busy,
                ..
            })
        ),
        "{response:?}"
    );
}

#[test]
fn one_record_first_page_returns_before_the_second_adapter_read() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset();
    let _release = Release;
    gate().control.lock().unwrap().block_at = Some(1);
    let client = Arc::new(support::Client::new::<Fixture>());
    let (mount, root) = client.root();
    let child = client.clone();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        send.send(child.call(Request::ListBegin {
            mount,
            directory: root.id,
            max_entries: 1,
        }))
        .unwrap();
    });
    let response = match receive.recv_timeout(Duration::from_secs(3)) {
        Ok(response) => response,
        Err(error) => {
            release();
            worker.join().unwrap();
            panic!("first page waited for the blocked second record: {error}");
        }
    };
    worker.join().unwrap();
    let Response::Page {
        cursor,
        entries,
        page: 0,
        done: false,
    } = response
    else {
        panic!("{response:?}")
    };
    assert_eq!(entries.len(), 1);
    assert!(!gate().control.lock().unwrap().entered);
    assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
}

#[test]
fn close_and_cancellation_keep_global_slots_until_blocked_os_call_finishes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let _release = Release;
    let one = Arc::new(support::Client::new::<Fixture>());
    let two = support::Client::new::<Fixture>();
    let three = support::Client::new::<Fixture>();
    let (mount, root) = one.root();
    let (m2, r2) = two.root();
    let (m3, r3) = three.root();
    let first = begin(&one, mount, root.id);
    let second = begin(&two, m2, r2.id);
    busy(three.call(Request::ListBegin {
        mount: m3,
        directory: r3.id,
        max_entries: 1,
    }));
    gate().control.lock().unwrap().block_at = Some(1);
    let child = one.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = std::thread::spawn(move || {
        child.invoke(
            Request::ListNext {
                cursor: first,
                expected_page: 1,
                max_entries: 1,
            },
            &|| worker_cancel.load(Ordering::Acquire),
        )
    });
    wait_for(|c| c.entered);
    cancel.store(true, Ordering::Release);
    assert_eq!(
        one.call(Request::CloseList { cursor: first }),
        Response::Unit
    );
    wait_for(|c| c.cancel_observed);
    busy(three.call(Request::ListBegin {
        mount: m3,
        directory: r3.id,
        max_entries: 1,
    }));
    busy(three.call(Request::Activate {
        mount: m3,
        node: r3.id,
    }));
    assert!(matches!(
        three.call(Request::Stat {
            mount: m3,
            node: r3.id
        }),
        Response::Node(_)
    ));
    assert!(matches!(
        three.call(Request::ResolveText {
            text: "manual-server".into()
        }),
        Response::Resolved { .. }
    ));
    release();
    assert!(matches!(
        worker.join().unwrap().unwrap(),
        Response::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    let third = begin(&three, m3, r3.id);
    two.call(Request::CloseList { cursor: second });
    three.call(Request::CloseList { cursor: third });
}

#[test]
fn blocked_cursor_destructor_retains_global_slot_without_catalog_lock() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let _release = Release;
    let client = Arc::new(support::Client::new::<Fixture>());
    let (mount, root) = client.root();
    let first = begin(&client, mount, root.id);
    let second = begin(&client, mount, root.id);
    gate().control.lock().unwrap().block_drop = true;
    let child = client.clone();
    let worker = std::thread::spawn(move || child.call(Request::CloseList { cursor: first }));
    wait_for(|c| c.drop_entered);
    busy(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 1,
    }));
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: root.id
        }),
        Response::Node(_)
    ));
    gate().control.lock().unwrap().release_drop = true;
    gate().changed.notify_all();
    assert_eq!(worker.join().unwrap(), Response::Unit);
    let replacement = begin(&client, mount, root.id);
    client.call(Request::CloseList { cursor: second });
    client.call(Request::CloseList {
        cursor: replacement,
    });
}

#[test]
fn refresh_rejects_inflight_page_and_prevents_old_epoch_cache_repopulation() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    let _release = Release;
    let client = Arc::new(support::Client::new::<Fixture>());
    let (mount, root) = client.root();
    gate().control.lock().unwrap().block_at = Some(0);
    let child = client.clone();
    let node = root.id;
    let worker = std::thread::spawn(move || {
        child.call(Request::ListBegin {
            mount,
            directory: node,
            max_entries: 128,
        })
    });
    wait_for(|c| c.entered);
    assert_eq!(
        client.call(Request::Refresh {
            mount,
            node: root.id
        }),
        Response::Unit
    );
    wait_for(|c| c.cancel_observed);
    release();
    assert!(matches!(
        worker.join().unwrap(),
        Response::Failed(Failure {
            code: ErrorCode::StaleReference,
            ..
        })
    ));
    for _ in 0..2 {
        let Response::Page {
            cursor,
            done,
            entries,
            ..
        } = client.call(Request::ListBegin {
            mount,
            directory: root.id,
            max_entries: 128,
        })
        else {
            panic!()
        };
        assert!(done);
        assert_eq!(entries[0].display, "Generation 2");
        client.call(Request::CloseList { cursor });
    }
    assert_eq!(BEGINS.load(Ordering::SeqCst), 2);
}
