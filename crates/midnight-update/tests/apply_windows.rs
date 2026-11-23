//! Windows: a file that another process holds open without delete sharing (as a
//! running program's files can be) blocks its rename. The updater tries again for a
//! moment, and when the file stays held it puts everything back.
use crate::apply::{Behaviour, FakeLauncher, fast, setup};
use midnight_update::apply::{self, Outcome, Timings};
use std::{cell::RefCell, fs::OpenOptions, os::windows::fs::OpenOptionsExt, time::Duration};

fn held(path: &std::path::Path) -> std::fs::File {
    // Share nothing: no rename, delete or write while this handle is open.
    OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[test]
fn a_file_released_within_the_retry_time_is_moved() {
    let setup = setup("me.exe");
    let handle = held(&setup.installation.join("viewers/old/plugin.dll"));
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(handle);
    });
    let launcher = FakeLauncher {
        installation: setup.installation.clone(),
        behaviour: Behaviour::Confirms,
        launched: RefCell::new(Vec::new()),
        saw_ready: RefCell::new(false),
    };
    let timings = Timings {
        rename: Duration::from_secs(5),
        ..fast()
    };
    let outcome = apply::apply(&setup.request, &launcher, timings);
    release.join().unwrap();
    assert!(matches!(outcome, Outcome::Updated { .. }), "{outcome:?}");
    assert_eq!(
        std::fs::read_to_string(setup.installation.join("viewers/old/plugin.dll")).unwrap(),
        "new plugin"
    );
}

#[test]
fn a_file_that_stays_held_leaves_the_previous_version_in_place() {
    let setup = setup("me.exe");
    let handle = held(&setup.installation.join("viewers/old/plugin.dll"));
    let launcher = FakeLauncher {
        installation: setup.installation.clone(),
        behaviour: Behaviour::Confirms,
        launched: RefCell::new(Vec::new()),
        saw_ready: RefCell::new(false),
    };
    let outcome = apply::apply(&setup.request, &launcher, fast());
    drop(handle);
    match outcome {
        Outcome::NotApplied { reason } => assert!(reason.contains("moved aside"), "{reason}"),
        other => panic!("expected no change, got {other:?}"),
    }
    // Everything that had moved came back; the staged copy is still there.
    assert_eq!(
        std::fs::read_to_string(setup.installation.join("me.exe")).unwrap(),
        "executable 0.5.0"
    );
    assert!(setup.installation.join("BUILD-INFO.json").is_file());
    assert!(setup.installation.join("obsolete.txt").is_file());
    assert!(setup.request.staged.join("me.exe").is_file());
    assert_eq!(launcher.launched.borrow()[0].content, "executable 0.5.0");
}

/// A running program's executable can be renamed on Windows; its folder's other
/// entries move around it. This is what lets the updater's own copy keep running.
#[test]
fn a_running_executable_can_be_renamed() {
    let folder = tempfile::tempdir().unwrap();
    let copy = folder.path().join("child.exe");
    std::fs::copy(std::env::var_os("ComSpec").unwrap(), &copy).unwrap();
    let mut child = std::process::Command::new(&copy)
        .args(["/c", "ping -n 3 127.0.0.1 >NUL"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let moved = folder.path().join("moved.exe");
    let result = std::fs::rename(&copy, &moved);
    let _ = child.kill();
    let _ = child.wait();
    result.unwrap();
}
