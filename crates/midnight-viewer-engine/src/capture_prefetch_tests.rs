//! Navigation captures only what the user is about to view; stale prefetches are renewed.
use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::{
    sync::{
        atomic::AtomicU64,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

// Elapses only when the actor fails to deliver; successful runs never wait for it.
const WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
struct Versioned {
    version: AtomicU64,
}

impl ContentSource for Versioned {
    fn length(&self) -> u64 {
        4
    }
    fn display_name(&self) -> String {
        "entry.txt".into()
    }
    fn version(&self) -> Vec<u8> {
        self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        Ok(b"text"[offset as usize..offset as usize + length as usize].to_vec())
    }
}

struct Fixture {
    sources: Vec<Arc<Versioned>>,
    captures: Receiver<usize>,
    wakes: Receiver<()>,
    capture: CaptureEntry,
    wake: Wake,
}

fn fixture(len: usize) -> Fixture {
    let sources: Vec<_> = (0..len).map(|_| Arc::default()).collect();
    let (captured, captures) = mpsc::channel();
    let (woken, wakes) = mpsc::sync_channel(64);
    let shared = sources.clone();
    let captured = Mutex::new(captured);
    Fixture {
        sources,
        captures,
        wakes,
        capture: Arc::new(move |index, _| {
            let _ = captured.lock().unwrap().send(index);
            CapturedFile::from_source(shared[index].clone()).map_err(|error| error.to_string())
        }),
        wake: Arc::new(move || woken.try_send(()).is_ok()),
    }
}

fn served(prefetch: &Prefetch, sequence: u64, wakes: &Receiver<()>) -> Captured {
    loop {
        if let Some(result) = prefetch.take(sequence) {
            return result;
        }
        wakes
            .recv_timeout(WAIT)
            .expect("the capture actor must wake its worker");
    }
}

fn spawn(fixture: &Fixture, len: usize, current: usize) -> Prefetch {
    let focused = (fixture.capture)(current, &Arc::default()).unwrap();
    assert_eq!(fixture.captures.recv().unwrap(), current);
    Prefetch::spawn(
        len,
        current,
        Some(focused),
        fixture.capture.clone(),
        Arc::default(),
        fixture.wake.clone(),
    )
    .unwrap()
}

#[test]
fn previous_file_is_ready_without_capturing_the_rest_of_a_large_folder() {
    let fixture = fixture(10_000);
    let prefetch = spawn(&fixture, 10_000, 5_000);
    let mut captured = vec![
        fixture.captures.recv_timeout(WAIT).unwrap(),
        fixture.captures.recv_timeout(WAIT).unwrap(),
    ];
    assert_eq!(captured, [5_001, 4_999], "next, then previous neighbor");
    let sequence = prefetch.request(4_999);
    assert!(served(&prefetch, sequence, &fixture.wakes).is_ok());
    captured.push(fixture.captures.recv_timeout(WAIT).unwrap());
    assert_eq!(captured, [5_001, 4_999, 4_998]);
    drop(prefetch);
    assert!(
        fixture.captures.try_recv().is_err(),
        "only the target and its neighbors are captured"
    );
}

#[test]
fn file_changed_after_prefetch_is_captured_again_instead_of_failing() {
    let fixture = fixture(3);
    let prefetch = spawn(&fixture, 3, 0);
    let mut prefetched = [
        fixture.captures.recv_timeout(WAIT).unwrap(),
        fixture.captures.recv_timeout(WAIT).unwrap(),
    ];
    prefetched.sort_unstable();
    assert_eq!(prefetched, [1, 2]);
    fixture.sources[1].version.store(7, Ordering::Release);
    let sequence = prefetch.request(1);
    let file = served(&prefetch, sequence, &fixture.wakes).unwrap();
    assert_eq!(
        fixture.captures.recv_timeout(WAIT).unwrap(),
        1,
        "recaptured"
    );
    assert_eq!(file.stable_version(), 7u64.to_le_bytes());
    assert_eq!(file.read_at(0, 4).unwrap(), b"text");
    let sequence = prefetch.request(2);
    let unchanged = served(&prefetch, sequence, &fixture.wakes).unwrap();
    assert_eq!(unchanged.stable_version(), 0u64.to_le_bytes());
    let sequence = prefetch.request(0);
    assert!(served(&prefetch, sequence, &fixture.wakes).is_ok());
    drop(prefetch);
    assert!(
        fixture.captures.try_recv().is_err(),
        "current cached identities are revalidated, not captured again"
    );
}

#[test]
fn a_newer_request_supersedes_an_older_result() {
    let fixture = fixture(5);
    let prefetch = spawn(&fixture, 5, 2);
    let older = prefetch.request(0);
    let newer = prefetch.request(4);
    assert!(prefetch.take(older).is_none());
    assert!(served(&prefetch, newer, &fixture.wakes).is_ok());
    assert!(prefetch.take(older).is_none());
}

struct DropSignal(mpsc::Sender<()>);
impl Drop for DropSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[test]
fn dropping_the_handle_stops_an_idle_actor() {
    let (dropped, released) = mpsc::channel();
    let signal = DropSignal(dropped);
    let (woken, _wakes) = mpsc::sync_channel(1);
    let prefetch = Prefetch::spawn(
        1,
        0,
        None,
        Arc::new(move |_, _| {
            let _ = &signal;
            Err("not captured".into())
        }),
        Arc::default(),
        Arc::new(move || woken.try_send(()).is_ok()),
    )
    .unwrap();
    drop(prefetch);
    released
        .recv_timeout(WAIT)
        .expect("a stopped actor releases its capture authority");
}

#[test]
fn closed_window_and_failing_captures_never_leave_the_worker_waiting() {
    let (woken, wakes) = mpsc::sync_channel(4);
    let wake: Wake = Arc::new(move || woken.try_send(()).is_ok());
    let failing = Prefetch::spawn(
        3,
        0,
        None,
        Arc::new(|index, _| panic!("provider fault for {index}")),
        Arc::default(),
        wake.clone(),
    )
    .unwrap();
    let sequence = failing.request(1);
    assert_eq!(
        served(&failing, sequence, &wakes).unwrap_err(),
        "File identity preparation failed."
    );
    drop(failing);
    let closed = Prefetch::spawn(
        3,
        0,
        None,
        Arc::new(|_, _| panic!("a closed window captures nothing")),
        Arc::new(AtomicBool::new(true)),
        wake,
    )
    .unwrap();
    let sequence = closed.request(1);
    assert_eq!(served(&closed, sequence, &wakes).unwrap_err(), STOPPED);
}
