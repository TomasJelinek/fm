//! Detection of a Chromium sandbox that AppArmor denies (`browser_sandbox_linux.rs`), with
//! the system's answers injected.
use super::*;

const DENIED_MAPPING: &[u8] =
    b"unshare: write failed /proc/self/uid_map: Operation not permitted\n";

fn not_called<T>() -> T {
    panic!("a later check ran although an earlier one already decided")
}

/// The system files `expected` reads, from `(path, content)` pairs; any other is missing.
fn system(files: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    let files = files.to_vec();
    move |path| {
        files
            .iter()
            .find(|(file, _)| *file == path)
            .map(|(_, content)| (*content).to_owned())
    }
}

#[test]
fn namespaces_are_checked_only_where_something_may_deny_them_without_a_setuid_helper() {
    let restricted = [(RESTRICTION, "1\n")];
    assert_eq!(
        expected(system(&restricted), || false),
        Namespaces::Restricted
    );
    assert_eq!(
        expected(system(&restricted), || true),
        Namespaces::Unchecked
    );
    // Systems without any restriction never look for the setuid helper.
    for files in [
        &[
            (RESTRICTION, "0\n"),
            (USERNS_CLONE, "1\n"),
            (MAX_NAMESPACES, "15000\n"),
        ][..],
        &[][..],
    ] {
        assert_eq!(expected(system(files), not_called), Namespaces::Unchecked);
    }
}

#[test]
fn a_kernel_switch_that_disables_namespaces_comes_before_apparmor() {
    let clone = Namespaces::Disabled("kernel.unprivileged_userns_clone");
    let limit = Namespaces::Disabled("user.max_user_namespaces");
    for (files, found) in [
        (&[(USERNS_CLONE, "0\n")][..], clone),
        (&[(USERNS_CLONE, "0\n"), (RESTRICTION, "1\n")][..], clone),
        (&[(MAX_NAMESPACES, "0\n"), (RESTRICTION, "1\n")][..], limit),
        (&[(USERNS_CLONE, "1\n"), (MAX_NAMESPACES, "0\n")][..], limit),
    ] {
        assert_eq!(expected(system(files), || false), found, "{files:?}");
        // A setuid-root helper replaces the namespaces.
        assert_eq!(expected(system(files), || true), Namespaces::Unchecked);
    }
}

#[test]
fn only_a_browser_without_any_profile_whose_probe_is_refused_is_denied() {
    let restricted = |label: Option<&'static str>, probe: fn() -> Probe| {
        decide(Namespaces::Restricted, || label.map(str::to_owned), probe)
    };
    assert_eq!(
        restricted(Some("unconfined\n"), || Probe::Denied),
        Some(Denial::AppArmor)
    );
    let others: [fn() -> Probe; 3] = [|| Probe::Allowed, || Probe::Exhausted, || Probe::Unknown];
    for probe in others {
        assert_eq!(restricted(Some("unconfined\n"), probe), None);
    }
    // The installed profile, or one the application itself runs under, allows namespaces.
    for label in [
        Some("midnight-explorer-html-viewer (unconfined)\n"),
        Some("other-app (unconfined)\n"),
        Some("other-app (enforce)\n"),
        None,
    ] {
        assert_eq!(restricted(label, not_called), None, "{label:?}");
    }
    assert_eq!(decide(Namespaces::Unchecked, not_called, not_called), None);
}

#[test]
fn a_disabling_kernel_switch_is_confirmed_by_the_probe_whatever_the_label() {
    let disabled = Namespaces::Disabled("kernel.unprivileged_userns_clone");
    let refused: [fn() -> Probe; 2] = [|| Probe::Denied, || Probe::Exhausted];
    for probe in refused {
        assert_eq!(
            decide(disabled, not_called, probe),
            Some(Denial::Disabled("kernel.unprivileged_userns_clone"))
        );
    }
    let allowed: [fn() -> Probe; 2] = [|| Probe::Allowed, || Probe::Unknown];
    for probe in allowed {
        assert_eq!(decide(disabled, not_called, probe), None);
    }
}

#[test]
fn a_setuid_sandbox_must_belong_to_root() {
    assert!(setuid_by_root(0, 0o104_755));
    assert!(!setuid_by_root(1000, 0o104_755));
    assert!(!setuid_by_root(0, 0o100_755));
}

#[test]
fn only_a_refused_mapping_counts_as_denied() {
    assert_eq!(classify(false, DENIED_MAPPING), Probe::Denied);
    assert_eq!(classify(true, b""), Probe::Allowed);
    assert_eq!(
        classify(false, b"unshare: unshare failed: No space left on device\n"),
        Probe::Exhausted
    );
    assert_eq!(
        classify(false, b"unshare: failed to execute /usr/bin/true\n"),
        Probe::Unknown
    );
}

#[test]
fn a_disabled_kernel_switch_names_the_setting_and_offers_no_profile() {
    let temp = tempfile::tempdir().unwrap();
    let executable = installation(temp.path(), Some(SCRIPT));
    let error = unavailable(
        &executable,
        Denial::Disabled("kernel.unprivileged_userns_clone"),
    );
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let inner = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<crate::browser::SandboxUnavailable>())
        .expect("the error is typed for the host");
    assert_eq!(
        inner.refusal(),
        &SandboxRefusal::NamespacesDisabled {
            setting: "kernel.unprivileged_userns_clone"
        }
    );
    let message = inner.to_string();
    assert!(message.starts_with(inner.refusal().cause()), "{message}");
    assert!(
        message.contains("(kernel.unprivileged_userns_clone is 0.)")
            && message.contains("An AppArmor profile does not help here.")
            && !message.contains(SCRIPT)
            && !message.contains("sudo"),
        "{message}"
    );
}

/// `<root>/viewers/midnight-viewer-html/midnight-viewer-html`, with the installer at
/// `script` below `root` when given.
fn installation(root: &Path, script: Option<&str>) -> PathBuf {
    let package = root.join("viewers/midnight-viewer-html");
    fs::create_dir_all(&package).unwrap();
    let executable = package.join("midnight-viewer-html");
    fs::write(&executable, b"").unwrap();
    if let Some(script) = script {
        let script = root.join(script);
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(script, b"").unwrap();
    }
    executable
}

const CAUSE: &str = "The HTML viewer needs permission for Chromium's sandbox on this system \
                     (Ubuntu restricts unprivileged user namespaces). ";

fn refusal(executable: &Path) -> SandboxRefusal {
    SandboxRefusal::UserNamespaces {
        executable: executable.to_owned(),
    }
}

fn generic() -> String {
    format!(
        "{CAUSE}An administrator can allow it once by installing the package's AppArmor \
         profile for this installation with {SCRIPT} (see README.txt and \
         system-requirements.md)."
    )
}

/// The real file types and modes, as if root owned every path and no group or other
/// user could write to it: a root-owned installation such as /opt/midnight-explorer.
fn as_root(path: &Path) -> Option<Ownership> {
    fs::symlink_metadata(path).ok().map(|metadata| Ownership {
        owner: 0,
        mode: metadata.mode() & !0o022,
    })
}

#[test]
fn a_root_owned_installation_names_its_installer_with_sudo() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();

    let package = base.join("package");
    let executable = installation(&package, Some(SCRIPT));
    let expected = format!("{CAUSE}Run once: sudo {}/{SCRIPT}", package.display());
    assert_eq!(
        message_with(&executable, &refusal(&executable), as_root),
        expected
    );

    // A development build: <checkout>/_build/<profile>/viewers/..., installer in scripts/.
    let checkout = base.join("checkout");
    fs::create_dir_all(checkout.join("scripts")).unwrap();
    fs::write(checkout.join("scripts").join(SCRIPT), b"").unwrap();
    let executable = installation(&checkout.join("_build/debug"), None);
    let expected = format!(
        "{CAUSE}Run once: sudo {}/scripts/{SCRIPT}",
        checkout.display()
    );
    assert_eq!(
        message_with(&executable, &refusal(&executable), as_root),
        expected
    );

    let spaced = base.join("my apps");
    let executable = installation(&spaced, Some(SCRIPT));
    let expected = format!("{CAUSE}Run once: sudo '{}/{SCRIPT}'", spaced.display());
    assert_eq!(
        message_with(&executable, &refusal(&executable), as_root),
        expected
    );

    // The command names the installer itself, not a link to it.
    let linked = base.join("linked");
    std::os::unix::fs::symlink(&package, &linked).unwrap();
    let executable = linked.join("viewers/midnight-viewer-html/midnight-viewer-html");
    let expected = format!("{CAUSE}Run once: sudo {}/{SCRIPT}", package.display());
    assert_eq!(
        message_with(&executable, &refusal(&executable), as_root),
        expected
    );

    // Without an installer beside it, the message still names the script and the cause.
    let executable = installation(&base.join("copied"), None);
    assert_eq!(
        message_with(&executable, &refusal(&executable), as_root),
        generic()
    );

    // An installer that is a link could name any file, even one that only root can
    // change: it is never suggested, and neither is its target.
    let elsewhere = base.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("program"), b"").unwrap();
    let linked_script = base.join("linked script");
    let executable = installation(&linked_script, None);
    std::os::unix::fs::symlink(elsewhere.join("program"), linked_script.join(SCRIPT)).unwrap();
    assert_eq!(
        message_with(&executable, &refusal(&executable), as_root),
        generic()
    );
}

#[test]
fn an_installer_the_user_can_change_is_never_suggested_with_sudo() {
    // A temporary folder belongs to the user running the tests (or is world-writable).
    let temp = tempfile::tempdir().unwrap();
    let executable = installation(&temp.path().join("package"), Some(SCRIPT));
    assert_eq!(message(&executable, &refusal(&executable)), generic());
    let checkout = temp.path().join("checkout");
    fs::create_dir_all(checkout.join("scripts")).unwrap();
    fs::write(checkout.join("scripts").join(SCRIPT), b"").unwrap();
    let executable = installation(&checkout.join("_build/debug"), None);
    assert_eq!(message(&executable, &refusal(&executable)), generic());
}

#[test]
fn only_a_regular_file_that_only_root_can_change_is_trusted() {
    use std::collections::HashMap;
    const FILE: u32 = 0o100_000;
    const FOLDER: u32 = 0o040_000;
    let root = |mode| Ownership { owner: 0, mode };
    let trusted: HashMap<&str, Ownership> = HashMap::from([
        ("/", root(FOLDER | 0o755)),
        ("/opt", root(FOLDER | 0o755)),
        ("/opt/me", root(FOLDER | 0o755)),
        ("/opt/me/install.sh", root(FILE | 0o755)),
    ]);
    let decide = |changes: &[(&'static str, Ownership)], path: &str| {
        let mut table = trusted.clone();
        table.extend(changes.iter().copied());
        only_root_can_change(Path::new(path), &|path: &Path| {
            table.get(path.to_str().unwrap()).copied()
        })
    };
    assert!(decide(&[], "/opt/me/install.sh"));
    // The installer itself: another owner, group or other write, not a regular file.
    let user = Ownership {
        owner: 1000,
        mode: FILE | 0o755,
    };
    assert!(!decide(
        &[("/opt/me/install.sh", user)],
        "/opt/me/install.sh"
    ));
    for mode in [FILE | 0o775, FILE | 0o757, FOLDER | 0o755, 0o120_777] {
        assert!(
            !decide(&[("/opt/me/install.sh", root(mode))], "/opt/me/install.sh"),
            "{mode:o}"
        );
    }
    // Every folder above it, up to and including /.
    for folder in ["/opt/me", "/opt", "/"] {
        let user = Ownership {
            owner: 1000,
            mode: FOLDER | 0o755,
        };
        assert!(!decide(&[(folder, user)], "/opt/me/install.sh"), "{folder}");
        for mode in [FOLDER | 0o775, FOLDER | 0o777, FOLDER | 0o1777] {
            assert!(
                !decide(&[(folder, root(mode))], "/opt/me/install.sh"),
                "{folder} {mode:o}"
            );
        }
    }
    // A path that cannot be inspected, or a relative one, is not trusted.
    assert!(!decide(&[], "/opt/me/missing.sh"));
    assert!(!decide(&[], "opt/me/install.sh"));
}

#[test]
fn the_error_carries_the_message_and_the_typed_refusal_for_the_viewer() {
    let temp = tempfile::tempdir().unwrap();
    let executable = installation(temp.path(), Some(SCRIPT));
    let error = unavailable(&executable, Denial::AppArmor);
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let inner = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<crate::browser::SandboxUnavailable>())
        .expect("the error is typed for the host");
    assert_eq!(
        inner.to_string(),
        message(&executable, &refusal(&executable))
    );
    // The refused executable as launched, so that a host can install the profile for it.
    assert_eq!(inner.refusal(), &refusal(&executable));
    assert!(inner.to_string().starts_with(inner.refusal().cause()));
}
