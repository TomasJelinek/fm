//! The real Linux session: event-driven waits, its waker and clipboard ownership. The
//! X11 owner's own cases are in `file_clipboard_x11.rs`.
use super::*;
use crate::file_clipboard::Files;
use midnight_core::FileClipboardIntent;
use std::{fs, path::PathBuf, sync::Weak, time::Instant};

/// Elapses only when the event that should end a wait does not.
const FAILURE_WAIT: Duration = Duration::from_secs(30);

/// Whether a wait or loop that began at `started` ended well within `FAILURE_WAIT`.
fn prompt(started: Instant) -> bool {
    started.elapsed() < FAILURE_WAIT / 2
}

fn wake(session: &Session) -> &Wake {
    session.wake.as_ref().expect("the session has an eventfd")
}

/// Whether a wake is pending: the eventfd is readable now.
fn pending(wake: &Wake) -> bool {
    let mut descriptor = [PollFd::new(&*wake.0, PollFlags::IN)];
    poll(&mut descriptor, Some(&timespec(Duration::ZERO))).unwrap() == 1
}

/// The `/proc` stat file of the calling thread, which other threads can read.
fn thread_stat() -> PathBuf {
    // "/proc/thread-self" links to "<pid>/task/<tid>".
    Path::new("/proc")
        .join(fs::read_link("/proc/thread-self").unwrap())
        .join("stat")
}

/// Whether the thread of `stat` sleeps (state S), as it does blocked in `poll()`.
fn sleeping(stat: &Path) -> bool {
    fs::read_to_string(stat).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, fields)| fields.trim_start().starts_with('S'))
    })
}

fn file_list(path: &str) -> FileClipboard {
    FileClipboard {
        paths: vec![path.into()],
        intent: FileClipboardIntent::Copy,
    }
}

#[test]
fn session_open_does_not_require_a_display_or_text_owner() {
    // Also run with DISPLAY unset and an isolated, empty Wayland runtime.
    // Read transport selection owns capability errors, not session construction.
    let session = NativeClipboard.open().unwrap();
    assert!(
        !session.owns_clipboard(),
        "opening takes no clipboard ownership"
    );
    assert!(
        session.waker().is_some(),
        "the wake event does not depend on a display"
    );
}

#[test]
fn waker_ends_the_next_or_current_wait_and_outlives_its_session() {
    let mut session = Session::new();
    let waker = session
        .waker()
        .expect("Linux sessions are woken through an eventfd");
    // Wakes made before the wait keep the eventfd readable; they coalesce into one
    // wakeup that ends the next wait, which consumes all of them.
    waker();
    waker();
    let started = Instant::now();
    session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(prompt(started), "an earlier wake must end the next wait");
    assert!(
        !pending(wake(&session)),
        "one wait consumes coalesced wakes"
    );
    // A wake from another thread ends the wait in progress: that thread wakes once the
    // owner thread sleeps, in poll(). (A wake that wins the race ends the wait as well.)
    let owner = thread_stat();
    let remote = waker.clone();
    let other = std::thread::spawn(move || {
        let started = Instant::now();
        while !sleeping(&owner) && prompt(started) {
            std::thread::yield_now();
        }
        remote();
    });
    let started = Instant::now();
    session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(
        prompt(started),
        "a wake from another thread must end the wait"
    );
    other.join().unwrap();
    // Every waker keeps the eventfd open: a call after the session closed writes to an
    // eventfd nobody reads, and the last waker closes it.
    let event: Weak<OwnedFd> = Arc::downgrade(&wake(&session).0);
    drop(session);
    assert!(event.upgrade().is_some(), "a waker keeps the eventfd open");
    waker();
    drop(waker);
    assert!(
        event.upgrade().is_none(),
        "the last waker closes the eventfd"
    );
}

#[test]
fn session_without_an_x11_connection_owns_nothing_and_waits_for_its_waker() {
    // No X11 connection: the read-only Wayland transport, the one a session uses when
    // X11 cannot be opened, has no owner events to serve.
    let mut session = Session::new();
    session.files = Some(Files::Wayland);
    let refused = session.copy_files(&file_list("/tmp/source")).unwrap_err();
    assert_eq!(refused.kind, io::ErrorKind::Unsupported);
    assert!(
        !session.owns_clipboard(),
        "a refused copy takes no ownership"
    );
    let waker = session.waker().expect("a waker without X11");
    let other = std::thread::spawn(move || waker());
    let started = Instant::now();
    session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(prompt(started), "the waker alone must end the wait");
    other.join().unwrap();
}

#[test]
fn session_without_a_wake_event_offers_no_waker_and_waits_out_its_timeout() {
    // A session whose eventfd could not be created: the engine keeps its timed pump for
    // a session without a waker, and each wait still ends at its timeout.
    let mut session = Session {
        wake: None,
        ..Session::new()
    };
    assert!(session.waker().is_none());
    session.wait_pending(Duration::from_millis(1)).unwrap();
    session.files = Some(Files::Wayland);
    session.wait_pending(Duration::from_millis(1)).unwrap();
    assert!(!session.owns_clipboard());
}

#[test]
fn wait_timeouts_become_ppoll_timespecs_and_saturate() {
    let short = timespec(Duration::from_millis(1500));
    assert_eq!((short.tv_sec, short.tv_nsec), (1, 500_000_000));
    let zero = timespec(Duration::ZERO);
    assert_eq!((zero.tv_sec, zero.tv_nsec), (0, 0));
    assert_eq!(timespec(Duration::MAX).tv_sec, i64::MAX);
    // The kernel accepts the saturated timeout (ppoll rejects an invalid one with
    // EINVAL). The eventfd is signaled directly, not through the waker under test, so
    // that this wait ends at once even if the waker were broken.
    let wake = Wake::new().unwrap();
    rustix::io::write(&*wake.0, &1u64.to_ne_bytes()).unwrap();
    let started = Instant::now();
    assert!(!wait(Some(&wake), None, Duration::MAX).unwrap());
    assert!(prompt(started));
    assert!(!pending(&wake), "the wait consumed the wake");
}

/// Wait until the X server lists a CLIPBOARD owner. arboard sets it from its own
/// connection without a round trip, so a later request of another connection could
/// otherwise reach the server first.
fn await_clipboard_owner() {
    use x11rb::{NONE, protocol::xproto::ConnectionExt as _};
    let (connection, _) = x11rb::connect(None).unwrap();
    let clipboard = connection
        .intern_atom(false, b"CLIPBOARD")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let started = Instant::now();
    while connection
        .get_selection_owner(clipboard)
        .unwrap()
        .reply()
        .unwrap()
        .owner
        == NONE
    {
        assert!(prompt(started), "no CLIPBOARD owner appeared");
        std::thread::yield_now();
    }
}

/// Assert that the X11 tests run against a private display.
fn isolated_display() {
    assert_eq!(
        std::env::var("MIDNIGHT_CLIPBOARD_TEST_ISOLATED").as_deref(),
        Ok("1")
    );
    // arboard prefers Wayland data control, which would publish the test text on a real
    // desktop clipboard.
    assert!(
        std::env::var_os("WAYLAND_DISPLAY").is_none_or(|display| display.is_empty()),
        "unset WAYLAND_DISPLAY for the isolated clipboard tests"
    );
}

#[test]
#[ignore = "requires an isolated Xvfb DISPLAY, no WAYLAND_DISPLAY and MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1"]
fn x11_session_ownership_follows_its_text_and_file_owners() {
    isolated_display();
    let mut session = Session::new();
    // A read connects the X11 owner window but takes no ownership.
    assert!(
        session.read_files().is_err(),
        "the new display has no file list"
    );
    assert!(!session.owns_clipboard(), "reading takes no ownership");
    session.copy_text("midnight ownership test").unwrap();
    assert!(
        session.owns_clipboard(),
        "arboard serves the text: keep the session open"
    );
    await_clipboard_owner();
    session
        .copy_files(&file_list("/tmp/midnight-owned"))
        .unwrap();
    assert!(
        session.owns_clipboard(),
        "the X11 owner window serves the file list"
    );
    // A wake ends a wait that also watches the X11 connection.
    let waker = session.waker().expect("a waker with X11");
    waker();
    let started = Instant::now();
    session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(prompt(started), "the waker must end the wait");
    // Another client takes CLIPBOARD. Its SelectionClear, not the waker, ends the wait
    // and the ownership: the replaced text and file list no longer need this session.
    let taker = std::thread::spawn(|| {
        let mut other = Files::new().unwrap();
        other.copy(&file_list("/tmp/midnight-other")).unwrap();
        other
    });
    let started = Instant::now();
    while session.owns_clipboard() {
        assert!(prompt(started), "losing ownership must end the wait");
        session.wait_pending(FAILURE_WAIT).unwrap();
    }
    assert!(prompt(started), "losing ownership must end the wait");
    drop(taker.join().unwrap());
}
