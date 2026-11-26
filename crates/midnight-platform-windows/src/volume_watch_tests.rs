use super::*;
use std::{
    path::Path,
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DBT_DEVICEQUERYREMOVE, DBT_DEVTYP_DEVICEINTERFACE, SendMessageW,
};

fn header(kind: u32) -> DEV_BROADCAST_HDR {
    DEV_BROADCAST_HDR {
        dbch_size: size_of::<DEV_BROADCAST_HDR>() as u32,
        dbch_devicetype: kind,
        dbch_reserved: 0,
    }
}

#[test]
fn only_volume_arrival_and_removal_are_drive_changes() {
    let volume = header(DBT_DEVTYP_VOLUME);
    let interface = header(DBT_DEVTYP_DEVICEINTERFACE);
    assert!(is_volume_change(DBT_DEVICEARRIVAL as WPARAM, &volume));
    assert!(is_volume_change(
        DBT_DEVICEREMOVECOMPLETE as WPARAM,
        &volume
    ));
    assert!(!is_volume_change(DBT_DEVICEQUERYREMOVE as WPARAM, &volume));
    assert!(!is_volume_change(DBT_DEVICEARRIVAL as WPARAM, &interface));
    assert!(!is_volume_change(DBT_DEVICEARRIVAL as WPARAM, ptr::null()));
}

type Hints = Arc<Mutex<Vec<(Instant, Source)>>>;

fn recording() -> (Hints, Arc<dyn Fn(Source) + Send + Sync>) {
    let hints: Hints = Arc::default();
    let sink = hints.clone();
    (
        hints,
        Arc::new(move |source| sink.lock().unwrap().push((Instant::now(), source))),
    )
}

#[test]
fn a_volume_broadcast_hints_once_and_the_subscription_stops_its_thread() {
    let (hints, callback) = recording();
    let watch = watch_sources(callback).unwrap();
    let volume = header(DBT_DEVTYP_VOLUME);
    // SAFETY: the window lives until `watch` is dropped; the header outlives the call.
    unsafe {
        SendMessageW(
            watch.window as HWND,
            WM_DEVICECHANGE,
            DBT_DEVICEARRIVAL as WPARAM,
            &volume as *const _ as LPARAM,
        );
        SendMessageW(
            watch.window as HWND,
            WM_DEVICECHANGE,
            DBT_DEVICEQUERYREMOVE as WPARAM,
            &volume as *const _ as LPARAM,
        );
    }
    assert_eq!(
        hints
            .lock()
            .unwrap()
            .iter()
            .map(|(_, source)| *source)
            .collect::<Vec<_>>(),
        [Source::Broadcast]
    );
    drop(watch);
    let after = hints.lock().unwrap().len();
    std::thread::sleep(POLL + Duration::from_millis(200));
    assert_eq!(hints.lock().unwrap().len(), after);
}

/// A `subst` drive letter of this test's own folder, removed again on drop.
struct Subst(char);

impl Subst {
    fn define(folder: &Path) -> Self {
        // SAFETY: GetLogicalDrives takes no pointers and returns a bitmask.
        let used = unsafe { GetLogicalDrives() };
        let letter = (b'M'..=b'Y')
            .rev()
            .find(|letter| used & (1 << (letter - b'A')) == 0)
            .expect("a free drive letter") as char;
        let status = Command::new("subst.exe")
            .arg(format!("{letter}:"))
            .arg(folder)
            .status()
            .unwrap();
        assert!(status.success(), "subst {letter}: failed");
        Self(letter)
    }

    fn remove(&mut self) {
        if self.0 != '\0' {
            let _ = Command::new("subst.exe")
                .arg(format!("{}:", self.0))
                .arg("/d")
                .status();
            self.0 = '\0';
        }
    }
}

impl Drop for Subst {
    fn drop(&mut self) {
        self.remove();
    }
}

fn first_hint_after(hints: &Hints, start: Instant) -> Option<(Duration, Source)> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some((at, source)) = hints
            .lock()
            .unwrap()
            .iter()
            .find(|(at, _)| *at >= start)
            .copied()
        {
            return Some((at - start, source));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

/// Adds and removes a drive letter of this session with `subst`, which another
/// program running in the same session sees for a moment. Run it explicitly:
/// `cargo test -p midnight-platform-windows --lib volume_watch -- --ignored --nocapture`.
#[test]
#[ignore = "defines a temporary subst drive letter in the user's session"]
fn a_subst_drive_letter_is_reported_when_added_and_removed() {
    let folder = tempfile::tempdir().unwrap();
    let (hints, callback) = recording();
    let watch = watch_sources(callback).unwrap();
    let added = Instant::now();
    let mut subst = Subst::define(folder.path());
    let defined = Instant::now();
    let arrival = first_hint_after(&hints, added);
    let removed = Instant::now();
    subst.remove();
    let released = Instant::now();
    let removal = first_hint_after(&hints, removed);
    drop(watch);
    println!(
        "subst add: command {:?}, first hint after {:?}; subst /d: command {:?}, first hint after {:?}",
        defined - added,
        arrival,
        released - removed,
        removal,
    );
    let all: Vec<_> = hints.lock().unwrap().iter().map(|(_, s)| *s).collect();
    println!("hints: {all:?}");
    assert!(arrival.is_some_and(|(latency, _)| latency < POLL + Duration::from_millis(500)));
    assert!(removal.is_some_and(|(latency, _)| latency < POLL + Duration::from_millis(500)));
}
