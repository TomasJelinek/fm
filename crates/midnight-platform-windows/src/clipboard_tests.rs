//! The real Windows session: event-driven waits, its waker and clipboard ownership.
use super::*;
use crate::file_clipboard::Files;
use midnight_core::FileClipboardIntent;
use std::time::Instant;
use windows_sys::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
        PM_NOREMOVE, PostThreadMessageW, SendNotifyMessageW, WM_APP, WM_DESTROYCLIPBOARD,
    },
};

/// Elapses only when the event that should end a wait does not.
const FAILURE_WAIT: Duration = Duration::from_secs(30);

fn session() -> Session {
    Session {
        text: arboard::Clipboard::new().unwrap(),
        files: None,
        wake: Arc::new(wake_event().unwrap()),
    }
}

/// Whether the calling thread's message queue is empty; creates the queue if needed.
fn queue_is_empty() -> bool {
    let mut message = MSG::default();
    // SAFETY: PM_NOREMOVE only inspects (and on first use creates) this thread's queue.
    unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_NOREMOVE) == 0 }
}

#[test]
fn waker_ends_the_next_or_current_wait_and_outlives_its_session() {
    let mut session = session();
    let waker = session
        .waker()
        .expect("Windows sessions are woken through an event");
    // A wake made before the wait starts stays signaled until the wait consumes it.
    waker();
    let started = Instant::now();
    session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(
        started.elapsed() < FAILURE_WAIT,
        "an earlier wake must end the next wait"
    );
    // A wake from another thread ends the wait in progress, or the next one if it wins.
    let remote = waker.clone();
    let other = std::thread::spawn(move || remote());
    let started = Instant::now();
    session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(
        started.elapsed() < FAILURE_WAIT,
        "a wake from another thread must end the wait"
    );
    other.join().unwrap();
    drop(session);
    // Every waker keeps the event open: a call after the session closed is harmless.
    waker();
}

#[test]
fn queued_owner_thread_messages_end_a_wait_and_are_drained() {
    let mut session = session();
    assert!(queue_is_empty());
    // SAFETY: GetCurrentThreadId has no preconditions.
    let owner = unsafe { GetCurrentThreadId() };
    // SAFETY: posts a payload-free application message to a thread that has a queue.
    let poster =
        std::thread::spawn(move || unsafe { PostThreadMessageW(owner, WM_APP, 0, 0) } != 0);
    let started = Instant::now();
    session.wait_pending(FAILURE_WAIT).unwrap();
    assert!(
        started.elapsed() < FAILURE_WAIT,
        "a queued message must end the wait"
    );
    assert!(poster.join().unwrap(), "PostThreadMessageW failed");
    // Dispatched, not left queued to end every later MWMO_INPUTAVAILABLE wait at once.
    assert!(queue_is_empty());
}

#[test]
fn wm_destroyclipboard_ends_the_wait_and_the_ownership() {
    let mut session = session();
    assert!(!session.owns_clipboard(), "no file window, no ownership");
    let files = Files::new().unwrap();
    assert!(!files.owns_clipboard(), "a new window owns nothing");
    // What a successful EmptyClipboard in `Files::copy` records, without the system clipboard.
    files.mark_owner();
    let window = files.window() as isize;
    session.files = Some(files);
    assert!(session.owns_clipboard());
    // Another owner emptying the clipboard makes Windows send WM_DESTROYCLIPBOARD.
    let sender = std::thread::spawn(move || {
        // SAFETY: queues a payload-free message for a live window of the waiting thread.
        unsafe { SendNotifyMessageW(window as HWND, WM_DESTROYCLIPBOARD, 0, 0) != 0 }
    });
    let started = Instant::now();
    while session.owns_clipboard() {
        assert!(
            started.elapsed() < FAILURE_WAIT,
            "WM_DESTROYCLIPBOARD must end the wait"
        );
        session.wait_pending(FAILURE_WAIT).unwrap();
    }
    assert!(sender.join().unwrap(), "SendNotifyMessageW failed");
    assert!(started.elapsed() < FAILURE_WAIT);
}

#[test]
fn wait_timeouts_round_up_and_stay_finite() {
    assert_eq!(wait_milliseconds(Duration::ZERO), 0);
    assert_eq!(wait_milliseconds(Duration::from_nanos(1)), 1);
    assert_eq!(wait_milliseconds(Duration::from_millis(10)), 10);
    assert_eq!(wait_milliseconds(Duration::from_micros(10_001)), 11);
    assert_eq!(wait_milliseconds(Duration::MAX), u32::MAX - 1);
}

#[test]
#[ignore = "manual: publishes a file name on the real clipboard, then lets another owner replace it"]
fn ownership_follows_the_real_clipboard_owner() {
    let mut session = session();
    let previous = session.read_text().ok();
    assert!(!session.owns_clipboard(), "reading takes no ownership");
    let files = FileClipboard {
        paths: vec![std::env::temp_dir().join("midnight ownership 文档.txt")],
        intent: FileClipboardIntent::Copy,
    };
    session.copy_files(&files).unwrap();
    assert!(session.owns_clipboard(), "publishing files takes ownership");
    assert_eq!(session.read_files().unwrap().paths, files.paths);
    // Another owner (arboard text on another thread) empties the clipboard; Windows sends
    // WM_DESTROYCLIPBOARD to our window, which ends the wait without the waker.
    let other = std::thread::spawn(|| {
        NativeClipboard
            .open()
            .and_then(|mut other| other.copy_text("another clipboard owner"))
            .is_ok()
    });
    let started = Instant::now();
    while session.owns_clipboard() {
        assert!(
            started.elapsed() < FAILURE_WAIT,
            "losing ownership must end the wait"
        );
        session.wait_pending(FAILURE_WAIT).unwrap();
    }
    assert!(other.join().unwrap(), "the other owner could not publish");
    if let Some(text) = previous {
        session.copy_text(&text).unwrap();
    }
}
