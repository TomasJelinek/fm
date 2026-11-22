//! Web and mail links go to xdg-open, else GIO, from the system's own folder, as one
//! literal argument and never through a shell. No test starts a helper.
use super::*;
use std::{
    cell::RefCell,
    ffi::OsStr,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn link(text: &str) -> ExternalUrl {
    ExternalUrl::parse(text).unwrap()
}

/// A stand-in program at `path` with the permission bits `mode`; it is never run.
fn fake_program(path: &Path, mode: u32) {
    fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// A helper that is still running.
struct Pending(Arc<AtomicUsize>);
impl TerminalProcess for Pending {
    fn id(&self) -> u32 {
        78
    }
    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        Ok(None)
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn links_go_to_xdg_open_else_gio_from_the_system_folder_only() {
    // A fake root stands in for `/`: the fixed system paths are looked up beneath it.
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("usr/bin");
    fs::create_dir_all(&bin).unwrap();
    let probes = RefCell::new(Vec::new());
    let find = || {
        DesktopOpener::find_with(|program| {
            probes.borrow_mut().push(program.to_owned());
            executable_file(&root.path().join(program.strip_prefix("/").unwrap()))
        })
    };
    let missing = find().err().expect("nothing is installed");
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);

    fake_program(&bin.join("gio"), 0o755);
    assert!(matches!(find(), Ok(DesktopOpener::Gio)));
    // An xdg-open that cannot run, or that is a folder, is passed over.
    fake_program(&bin.join("xdg-open"), 0o644);
    assert!(matches!(find(), Ok(DesktopOpener::Gio)));
    fs::remove_file(bin.join("xdg-open")).unwrap();
    fs::create_dir(bin.join("xdg-open")).unwrap();
    assert!(matches!(find(), Ok(DesktopOpener::Gio)));
    fs::remove_dir(bin.join("xdg-open")).unwrap();
    // A link to a program that can run counts, as it does for documents.
    fake_program(&root.path().join("xdg-open-target"), 0o755);
    symlink(root.path().join("xdg-open-target"), bin.join("xdg-open")).unwrap();
    assert!(matches!(find(), Ok(DesktopOpener::Xdg)));
    fs::remove_file(bin.join("xdg-open")).unwrap();
    fake_program(&bin.join("xdg-open"), 0o755);
    assert!(matches!(find(), Ok(DesktopOpener::Xdg)));

    let probes = probes.into_inner();
    assert!(
        probes.iter().all(|probe| {
            probe == Path::new("/usr/bin/xdg-open") || probe == Path::new("/usr/bin/gio")
        }),
        "only the two fixed system paths are consulted: {probes:?}"
    );
}

#[test]
fn a_link_is_one_literal_argument_of_a_fixed_opener_and_never_shell_source() {
    let url = link("https://example.com/a b?q=\"x\"&run=$(touch nope);'`|<>");
    let text = url.as_str();
    assert!(text.bytes().all(|byte| byte.is_ascii_graphic()), "{text}");
    for (opener, program, arguments) in [
        (DesktopOpener::Xdg, "/usr/bin/xdg-open", vec![text]),
        (DesktopOpener::Gio, "/usr/bin/gio", vec!["open", "--", text]),
    ] {
        // The opener itself is started, never `sh -c`; the address is one argument.
        let child = open_url_with(&url, Ok(opener), |command| {
            assert_eq!(command.get_program(), OsStr::new(program));
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                arguments.iter().map(OsStr::new).collect::<Vec<_>>()
            );
            assert!(command.get_current_dir().is_none());
            assert_eq!(command.get_envs().count(), 0);
            Ok(Box::new(Pending(Arc::default())))
        })
        .unwrap();
        assert!(child.is_some(), "the launch was asked for");
    }
}

#[test]
fn a_link_launch_hands_the_live_helper_to_the_engine() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut child = open_url_with(
        &link("mailto:someone@example.com?subject=Hi"),
        Ok(DesktopOpener::Xdg),
        |_| Ok(Box::new(Pending(drops.clone()))),
    )
    .unwrap()
    .expect("the helper is returned for the Engine to reap");
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(child.id(), 78);
    assert!(child.try_wait().unwrap().is_none());
    drop(child);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn a_missing_opener_or_failed_launch_is_reported_for_the_link_without_a_path() {
    let url = link("https://example.com/");
    let missing = DesktopOpener::find_with(|_| false);
    let error = open_url_with(&url, missing, |_| panic!("nothing to launch"))
        .err()
        .expect("no opener is installed");
    assert_eq!(error.kind, io::ErrorKind::NotFound);
    assert_eq!(error.operation, "open link");
    assert!(error.path.as_os_str().is_empty());
    let error = open_url_with(&url, Ok(DesktopOpener::Gio), |_| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "launch denied",
        ))
    })
    .err()
    .expect("the launch failed");
    assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
    assert!(error.message.contains("launch denied"));
    assert_eq!(error.operation, "open link");
    assert!(error.path.as_os_str().is_empty());
}
