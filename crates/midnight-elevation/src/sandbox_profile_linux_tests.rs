//! The HTML viewer's AppArmor profile (`sandbox_profile_linux.rs`): its text against the
//! installer script's, the constant root command with a fake runner, and that command's
//! logic run unprivileged in a temporary folder. Nothing here runs pkexec or changes the
//! system's AppArmor policy.
use super::*;
use std::{
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

/// The repository's installer, whose `--print` writes the profile without installing it.
fn installer() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install-html-viewer-apparmor.sh")
}

fn bash() -> Option<&'static str> {
    ["/usr/bin/bash", "/bin/bash"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
}

/// `install-html-viewer-apparmor.sh --print <installation>`: its output, or `None` when
/// it refuses the installation.
fn printed(bash: &str, installation: &Path) -> Option<String> {
    let output = Command::new(bash)
        .arg(installer())
        .arg("--print")
        .arg(installation)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).unwrap())
}

/// `<root>/viewers/midnight-viewer-html/midnight-viewer-html`, created.
fn viewer(root: &Path) -> PathBuf {
    let executable = root.join(VIEWER);
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, b"").unwrap();
    executable
}

/// A source checkout with one development build; returns the build's viewer.
fn checkout(root: &Path, build: &str) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("Cargo.toml"), b"").unwrap();
    viewer(&root.join("_build").join(build))
}

/// A private temporary folder below the repository's `_build`: the system's temporary
/// folder is writable by every user, which refuses any installation below it.
fn base() -> (tempfile::TempDir, PathBuf) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/sandbox-profile");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::tempdir_in(fs::canonicalize(root).unwrap()).unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    (temp, base)
}

/// Whether the temporary folder itself is a path the profile can name, below folders that
/// only their owners can write to.
fn nameable_base(base: &Path) -> bool {
    let nameable = base.to_str().is_some_and(nameable);
    if !nameable {
        eprintln!("{} is no path AppArmor can name: skipped", base.display());
    }
    if let Some(shared) = shared(&up(base)) {
        eprintln!(
            "{} can be written by other users: skipped",
            shared.display()
        );
        return false;
    }
    nameable
}

#[test]
fn the_profile_is_the_one_the_installer_prints_for_the_same_installation() {
    let (_temp, base) = base();
    if !nameable_base(&base) {
        return;
    }
    let package = base.join("package");
    let spaced = base.join("my apps+@:x_y-1.0");
    let built = base.join("checkout");
    let copied = base.join("copied");
    let cases = [
        // An installation folder: its own viewer.
        (viewer(&package), package.clone()),
        // Spaces and the other characters AppArmor reads literally.
        (viewer(&spaced), spaced.clone()),
        // A development build: every build of the checkout.
        (checkout(&built, "debug"), built.clone()),
        // A build folder without its checkout: that build's own viewer.
        (
            viewer(&copied.join("_build/release")),
            copied.join("_build/release"),
        ),
    ];
    let linked = base.join("linked");
    std::os::unix::fs::symlink(&package, &linked).unwrap();
    // AppArmor attaches to the real path, which the installer resolves too.
    let cases = cases
        .into_iter()
        .chain([(linked.join(VIEWER), linked.clone())]);
    let bash = bash();
    if bash.is_none() {
        eprintln!("bash is unavailable: the installer's own output is not compared");
    }
    for (executable, installation) in cases {
        let profile = profile_for(&executable).unwrap();
        let expected = installation.canonicalize().unwrap();
        assert!(
            profile.text.contains(&format!("\"{}/", expected.display())),
            "{}",
            profile.text
        );
        if let Some(bash) = bash {
            assert_eq!(
                printed(bash, &installation).as_deref(),
                Some(profile.text.as_str()),
                "{}",
                installation.display()
            );
        }
    }
    let pattern = profile_for(&built.join("_build/debug").join(VIEWER)).unwrap();
    assert!(
        pattern
            .text
            .contains(&format!("\"{}/_build/*/{VIEWER}\"", built.display())),
        "{}",
        pattern.text
    );
}

#[test]
fn paths_apparmor_reads_as_patterns_are_refused_as_the_installer_refuses_them() {
    let (_temp, base) = base();
    if !nameable_base(&base) {
        return;
    }
    let bash = bash();
    let mut names: Vec<String> = (0x20u8..0x7f)
        .filter(|byte| *byte != b'/')
        .map(|byte| format!("x{}y", byte as char))
        .collect();
    names.extend(["x\ny", "x\ty", "xéy", "x\u{202e}y"].map(String::from));
    let mut accepted = String::new();
    for (index, name) in names.iter().enumerate() {
        let installation = base.join(index.to_string()).join(name);
        let executable = viewer(&installation);
        let profile = profile_for(&executable).ok();
        if profile.is_some() {
            accepted.extend(name.chars().nth(1));
        }
        if let Some(bash) = bash {
            assert_eq!(
                printed(bash, &installation),
                profile.map(|profile| profile.text),
                "{name:?}"
            );
        }
    }
    // Spelled out once, as the installer's help text names them.
    let mut expected: Vec<char> = " +-.:@_".chars().collect();
    expected.extend(('0'..='9').chain('A'..='Z').chain('a'..='z'));
    expected.sort_unstable();
    let mut accepted: Vec<char> = accepted.chars().collect();
    accepted.sort_unstable();
    assert_eq!(accepted, expected);
}

#[test]
fn a_path_through_a_folder_others_can_write_to_is_refused_as_the_installer_refuses_it() {
    let (_temp, base) = base();
    if !nameable_base(&base) {
        return;
    }
    let bash = bash();
    // A sticky folder anyone can write to, as /tmp: another user could create the
    // installation's path once it is gone. A group-writable one: the group could replace it,
    // unless that group is its owner's private group (user private groups, as CI runners
    // and Debian or Ubuntu desktops have), which both accept; see
    // `only_a_private_group_may_share_its_owners_write_access` for both cases.
    for (name, mode) in [("sticky", 0o1777), ("group", 0o775)] {
        let shared = base.join(name);
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(mode)).unwrap();
        let owner = fs::metadata(&shared).unwrap();
        let acl = || may_have_acl(&shared, true);
        let refused =
            !only_its_owner_writes(mode, owner.uid(), owner.gid(), &Accounts::read(), acl);
        let installation = shared.join("me");
        let executable = viewer(&installation);
        match profile_for(&executable) {
            Err(error) => assert!(
                refused
                    && error
                        .to_string()
                        .contains(&format!("can write to {}", shared.display())),
                "{name}: {error}"
            ),
            Ok(_) => assert!(!refused, "{name}: accepted"),
        }
        // Also the folders below the installation on the way to the viewer.
        let inner = base.join(format!("{name}-inner"));
        let inner_executable = viewer(&inner);
        fs::set_permissions(
            inner_executable.parent().unwrap(),
            fs::Permissions::from_mode(mode),
        )
        .unwrap();
        assert_eq!(profile_for(&inner_executable).is_err(), refused, "{name}");
        // A checkout's builds: its _build folder.
        let checkout_root = base.join(format!("{name}-checkout"));
        let built = checkout(&checkout_root, "debug");
        fs::set_permissions(
            checkout_root.join("_build"),
            fs::Permissions::from_mode(mode),
        )
        .unwrap();
        assert_eq!(profile_for(&built).is_err(), refused, "{name}");
        // The installer decides each path as the application does.
        if let Some(bash) = bash {
            for (folder, viewer) in [
                (&installation, &executable),
                (&inner, &inner_executable),
                (&checkout_root, &built),
            ] {
                assert_eq!(
                    printed(bash, folder),
                    profile_for(viewer).ok().map(|profile| profile.text),
                    "{}",
                    folder.display()
                );
            }
        }
    }
}

/// The application refuses the viewer `executable` because users other than its owner can
/// write to `shared`, and the installer refuses its `installation` too.
fn assert_refused(installation: &Path, executable: &Path, shared: &Path, case: &str) {
    match profile_for(executable) {
        Err(error) => assert!(
            error
                .to_string()
                .contains(&format!("can write to {},", shared.display())),
            "{case}: {error}"
        ),
        Ok(profile) => panic!("{case}: offered for {}", profile.attachment),
    }
    if let Some(bash) = bash() {
        assert_eq!(printed(bash, installation), None, "{case}: the installer");
    }
}

/// The application offers the profile for the viewer `executable`, and the installer
/// prints the same text for its `installation`.
fn assert_accepted(installation: &Path, executable: &Path, case: &str) {
    let profile = profile_for(executable).unwrap_or_else(|error| panic!("{case}: {error}"));
    if let Some(bash) = bash() {
        assert_eq!(
            printed(bash, installation),
            Some(profile.text),
            "{case}: the installer"
        );
    }
}

#[test]
fn every_path_down_to_the_viewers_is_checked_as_the_installer_checks_it() {
    let (_temp, base) = base();
    if !nameable_base(&base) {
        return;
    }
    // The profile of a checkout names the viewer of every build, so each build counts down
    // to its viewer: its folder, `viewers`, the viewer's folder and the viewer itself, and a
    // build folder without a viewer, where anybody could add one. Here anybody can write to
    // the path: a folder sticky like /tmp, or the viewer itself.
    let places = [
        "_build/debug",
        "_build/debug/viewers",
        "_build/debug/viewers/midnight-viewer-html",
        "_build/debug/viewers/midnight-viewer-html/midnight-viewer-html",
        "_build/empty",
    ];
    for (index, place) in places.into_iter().enumerate() {
        let root = base.join(format!("checkout-{index}"));
        let executable = checkout(&root, "debug");
        let shared = root.join(place);
        fs::create_dir_all(root.join("_build/empty")).unwrap();
        let mode = if shared.is_dir() { 0o1777 } else { 0o777 };
        fs::set_permissions(&shared, fs::Permissions::from_mode(mode)).unwrap();
        assert_refused(&root, &executable, &shared, place);
    }
    // An installation's viewer that anybody can replace.
    let installation = base.join("installation");
    let executable = viewer(&installation);
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o777)).unwrap();
    assert_refused(&installation, &executable, &executable, "installed viewer");
    // AppArmor attaches to the real path, which a link below _build leaves: what it names
    // does not count, nor does a file there, which can hold no build.
    let open = base.join("open");
    fs::create_dir(&open).unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o1777)).unwrap();
    let root = base.join("linked");
    let executable = checkout(&root, "debug");
    std::os::unix::fs::symlink(&open, root.join("_build/elsewhere")).unwrap();
    fs::create_dir(root.join("_build/release")).unwrap();
    std::os::unix::fs::symlink(&open, root.join("_build/release/viewers")).unwrap();
    let stamp = root.join("_build/stamp");
    fs::write(&stamp, b"").unwrap();
    fs::set_permissions(&stamp, fs::Permissions::from_mode(0o666)).unwrap();
    assert_accepted(&root, &executable, "links and files below _build");
}

/// A POSIX ACL in the kernel's xattr format (version 2, then tag, permissions and id of each
/// entry in tag order; the ids of unnamed entries are undefined): `user::rwx
/// user:<another>:<named> group::r-x mask::<named> other::r-x`, so `named` is what the other
/// user (nobody) may do and the mode's group bits, which show the mask, read `named`.
fn acl(owner: u32, named: u16) -> Vec<u8> {
    let other = if owner == 65534 { 65533 } else { 65534 };
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, permissions, id) in [
        (0x01u16, 7u16, u32::MAX),
        (0x02, named, other),
        (0x04, 5, u32::MAX),
        (0x10, named, u32::MAX),
        (0x20, 5, u32::MAX),
    ] {
        acl.extend_from_slice(&tag.to_le_bytes());
        acl.extend_from_slice(&permissions.to_le_bytes());
        acl.extend_from_slice(&id.to_le_bytes());
    }
    acl
}

/// Gives `path` the ACL `name` that lets another user do `named` (see `acl`); false, after
/// printing SKIP, where its filesystem has no POSIX ACLs.
fn set_acl(path: &Path, name: &CStr, named: u16) -> bool {
    let value = acl(fs::metadata(path).unwrap().uid(), named);
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a terminated path and attribute name, and a value that outlives the call.
    let set = unsafe {
        libc::setxattr(
            path.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if set == 0 {
        return true;
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EOPNOTSUPP) {
        eprintln!("SKIP ACL-shared folders: the temporary filesystem has no POSIX ACLs");
        return false;
    }
    panic!("setting a POSIX ACL failed: {error}");
}

/// Removes the ACL `name` of `path`; the mode keeps the former mask as its group bits.
fn remove_acl(path: &Path, name: &CStr) {
    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a terminated path and attribute name.
    let removed = unsafe { libc::removexattr(path.as_ptr(), name.as_ptr()) };
    assert_eq!(removed, 0, "{}", io::Error::last_os_error());
}

#[test]
fn a_private_group_folder_with_an_acl_is_refused_as_the_installer_refuses_it() {
    let (_temp, base) = base();
    if !nameable_base(&base) {
        return;
    }
    // A folder carries no ACL unless one is set; one that cannot be read counts as one.
    assert!(!may_have_acl(&base, true));
    assert!(may_have_acl(&base.join("missing"), true));
    let probe = fs::metadata(&base).unwrap();
    if !Accounts::read().private_group(probe.uid(), probe.gid()) {
        eprintln!(
            "SKIP ACL-shared folders: group {} is not this user's private group",
            probe.gid()
        );
        return;
    }
    // With a POSIX ACL the group bits are its mask: 0775 in the owner's private group then
    // lets the ACL's other user write as well, anywhere on the way to a viewer.
    let installation = |name: &str| {
        let root = base.join(name).join("me");
        let executable = viewer(&root);
        (root, executable)
    };
    let built = |name: &str| {
        let root = base.join(name);
        let executable = checkout(&root, "debug");
        (root, executable)
    };
    let mut cases = Vec::new();
    let (root, executable) = installation("above");
    let shared = root.parent().unwrap().to_owned();
    cases.push(("above an installation", shared, root, executable));
    let (root, executable) = installation("folder");
    let shared = executable.parent().unwrap().to_owned();
    cases.push(("the viewer's folder", shared, root, executable));
    let (root, executable) = installation("file");
    cases.push(("the viewer", executable.clone(), root, executable));
    let (root, executable) = built("build");
    cases.push(("a checkout's _build", root.join("_build"), root, executable));
    let (root, executable) = built("debug");
    let shared = root.join("_build/debug");
    cases.push(("a build folder", shared, root, executable));
    let (root, executable) = built("inner");
    let shared = executable.parent().unwrap().to_owned();
    cases.push(("a build's viewer folder", shared, root, executable));
    for (case, shared, root, executable) in cases {
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o775)).unwrap();
        if !set_acl(&shared, ACCESS_ACL, 7) {
            return;
        }
        assert_eq!(fs::metadata(&shared).unwrap().mode() & 0o7777, 0o775);
        assert_refused(&root, &executable, &shared, case);
        // The ACL alone made the difference: the same 0775 folder without it is accepted.
        remove_acl(&shared, ACCESS_ACL);
        assert_eq!(fs::metadata(&shared).unwrap().mode() & 0o7777, 0o775);
        assert_accepted(&root, &executable, case);
    }
    // A default ACL, which `ls -l` shows as well, counts the same.
    let (root, executable) = installation("default");
    let shared = root.parent().unwrap();
    fs::set_permissions(shared, fs::Permissions::from_mode(0o775)).unwrap();
    assert!(set_acl(shared, DEFAULT_ACL, 7));
    assert_refused(&root, &executable, shared, "a default ACL");
    // Without group write the mask lets nobody else write, whatever the ACL names.
    let (root, executable) = installation("read-only");
    let shared = root.parent().unwrap();
    assert!(set_acl(shared, ACCESS_ACL, 5));
    assert_eq!(fs::metadata(shared).unwrap().mode() & 0o7777, 0o755);
    assert_accepted(&root, &executable, "an ACL without write");
}

#[test]
fn only_a_private_group_may_share_its_owners_write_access() {
    let passwd = b"root:x:0:0::/root:/bin/sh\n\
        alice:x:1000:1000::/home/alice:/bin/sh\n\
        bob:x:1001:100::/home/bob:/bin/sh\n\
        carol:x:1002:1002::/home/carol:/bin/sh\n\
        dave:x:1003:1002::/home/dave:/bin/sh\n";
    let accounts = Accounts {
        passwd: passwd.to_vec(),
        group: b"root:x:0:\nalice:x:1000:\nusers:x:100:bob\ncarol:x:1002:\n".to_vec(),
    };
    // `acl` tells whether the folder carries an ACL.
    let writes = |mode: u32, uid: u32, gid: u32, acl: fn() -> bool| {
        only_its_owner_writes(mode, uid, gid, &accounts, acl)
    };
    // Alice's own group, named like her and listing nobody else, is private.
    assert!(accounts.private_group(1000, 1000));
    assert!(writes(0o40775, 1000, 1000, || false));
    // Unless the folder carries an ACL: its group bits are then the ACL's mask, which lets
    // the ACL's named users and groups write.
    assert!(!writes(0o40775, 1000, 1000, || true));
    // The ACL is read only when group write would admit the folder: the mask bounds every
    // named entry, so without group write nobody but the owner can write.
    let unread = || -> bool { unreachable!("the ACL decides nothing here") };
    // A shared group, a group that is also another user's primary group, or a group
    // other than the owner's primary group is not.
    assert!(!writes(0o40775, 1001, 100, unread));
    assert!(!accounts.private_group(1002, 1002));
    assert!(!accounts.private_group(1000, 1002));
    // Write access for others is never accepted, and owner-only folders always are.
    assert!(!writes(0o41777, 1000, 1000, unread));
    assert!(!writes(0o40757, 1000, 1000, unread));
    assert!(writes(0o40755, 1001, 100, unread));
    // A member listed besides the owner, or accounts that include a directory service.
    let listed = Accounts {
        passwd: passwd.to_vec(),
        group: b"alice:x:1000:alice,eve\n".to_vec(),
    };
    assert!(!listed.private_group(1000, 1000));
    let included = Accounts {
        passwd: [passwd.as_slice(), b"+::::::\n"].concat(),
        group: accounts.group.clone(),
    };
    assert!(!included.private_group(1000, 1000));
}

#[test]
fn the_profile_names_its_viewer_path_and_whether_others_than_root_can_change_it() {
    let (_temp, base) = base();
    if !nameable_base(&base) {
        return;
    }
    let package = base.join("package");
    let profile = profile_for(&viewer(&package)).unwrap();
    assert_eq!(
        profile.attachment(),
        format!("{}/{VIEWER}", package.display())
    );
    // A temporary folder of the user running the tests: changeable without root.
    assert!(profile.changeable());
    let checkout_root = base.join("checkout");
    let profile = profile_for(&checkout(&checkout_root, "release")).unwrap();
    assert_eq!(
        profile.attachment(),
        format!("{}/_build/*/{VIEWER}", checkout_root.display())
    );
    // Only folders that root owns are not changeable; `/` is one.
    assert!(!changeable(&up(Path::new("/"))).unwrap());
}

#[test]
fn only_the_html_viewer_of_an_installation_is_offered() {
    let (_temp, base) = base();
    if !nameable_base(&base) {
        return;
    }
    let trusted = || -> io::Result<()> { Ok(()) };
    let executable = viewer(&base.join("package"));
    assert!(offer(&executable, trusted).is_some());
    // pkexec that root alone cannot change offers nothing, whatever the installation.
    assert!(offer(&executable, || Err(io::Error::other("untrusted"))).is_none());
    // Another program, a missing viewer and a viewer outside viewers/ are not covered.
    let other = base.join("other/bin/browser");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    fs::write(&other, b"").unwrap();
    assert!(offer(&other, trusted).is_none());
    assert!(offer(&base.join("missing").join(VIEWER), trusted).is_none());
    let loose = base.join("loose/midnight-viewer-html/midnight-viewer-html");
    fs::create_dir_all(loose.parent().unwrap()).unwrap();
    fs::write(&loose, b"").unwrap();
    assert!(offer(&loose, trusted).is_none());
}

#[test]
fn a_checkout_pattern_covers_only_viewers_of_its_builds() {
    let pattern = format!("/src/fm/_build/*/{VIEWER}");
    assert!(covers(
        &pattern,
        &Path::new("/src/fm/_build/debug").join(VIEWER)
    ));
    assert!(covers(
        &pattern,
        &Path::new("/src/fm/_build/odd[name]").join(VIEWER)
    ));
    assert!(!covers(&pattern, &Path::new("/src/fm/_build").join(VIEWER)));
    assert!(!covers(
        &pattern,
        &Path::new("/src/fm/_build/a/b").join(VIEWER)
    ));
    assert!(!covers(
        &pattern,
        &Path::new("/src/other/_build/debug").join(VIEWER)
    ));
    let exact = format!("/opt/me/{VIEWER}");
    assert!(covers(&exact, &Path::new("/opt/me").join(VIEWER)));
    assert!(!covers(&exact, &Path::new("/opt/you").join(VIEWER)));
}

fn finished(code: i32, error: &str) -> Finished {
    Finished {
        status: ExitStatus::from_raw(code << 8),
        error: error.as_bytes().to_vec(),
    }
}

#[test]
fn polkit_runs_the_constant_root_command_with_the_profile_on_its_input() {
    let text = b"profile text";
    let mut seen = None;
    let result = install_with(
        text,
        &Cancellation::default(),
        || Ok(()),
        |command, input, _| {
            let arguments: Vec<_> = command.get_args().map(OsStr::to_owned).collect();
            let environment: Vec<_> = command
                .get_envs()
                .map(|(key, value)| (key.to_owned(), value.map(OsStr::to_owned)))
                .collect();
            seen = Some((
                command.get_program().to_owned(),
                arguments,
                environment,
                input.to_vec(),
            ));
            Ok(finished(0, ""))
        },
    );
    assert_eq!(result, Ok(()));
    let (program, arguments, environment, input) = seen.unwrap();
    assert_eq!(program, PKEXEC);
    assert_eq!(
        arguments,
        ["--disable-internal-agent", "/bin/sh", "-c", INSTALL]
    );
    // pkexec's own messages, which tell a missing agent apart, stay in English.
    assert_eq!(
        environment,
        [("LC_ALL".into(), Some("C".into()))],
        "nothing else of the application's environment is set for pkexec"
    );
    assert_eq!(input, text, "the profile reaches root only as input");
    // The root command names the profile's file and no installation or checkout path.
    assert!(INSTALL.contains(&format!("target=/etc/apparmor.d/{PROFILE}\n")));
    assert!(INSTALL.starts_with("PATH=/usr/sbin:/usr/bin:/sbin:/bin\n"));
}

#[test]
fn the_outcome_distinguishes_declined_authorization_from_failures() {
    let outcome = |result: io::Result<Finished>| {
        let mut result = Some(result);
        install_with(
            b"text",
            &Cancellation::default(),
            || Ok(()),
            |_, _, _| result.take().unwrap(),
        )
    };
    for code in [126, 127] {
        assert_eq!(
            outcome(Ok(finished(code, "Not authorized"))),
            Err(SandboxProfileError::Declined)
        );
    }
    // Without a polkit agent in the session, asking again can never succeed.
    let no_agent = "Error executing command as another user: No authentication agent found.\n";
    assert_eq!(
        outcome(Ok(finished(127, no_agent))),
        Err(SandboxProfileError::NoAgent)
    );
    // Only pkexec's own status 127 means that; the root command never exits so.
    assert!(matches!(
        outcome(Ok(finished(1, no_agent))),
        Err(SandboxProfileError::Failed(_))
    ));
    assert_eq!(
        outcome(Ok(finished(
            1,
            "parser: syntax error\n\tline 3\nAppArmor refused the profile.\n\u{1b}[0m"
        ))),
        Err(SandboxProfileError::Failed(
            "parser: syntax error line 3 AppArmor refused the profile. [0m".into()
        ))
    );
    assert_eq!(
        outcome(Ok(finished(3, ""))),
        Err(SandboxProfileError::Failed(
            "The installation ended with exit status: 3.".into()
        ))
    );
    let long = "e".repeat(ERROR_CHARS + 10);
    let Err(SandboxProfileError::Failed(text)) = outcome(Ok(finished(1, &long))) else {
        panic!("a long error is a failure");
    };
    assert_eq!(text.chars().count(), ERROR_CHARS + 1);
    assert!(text.ends_with('…'));
    let killed = Finished {
        status: ExitStatus::from_raw(9),
        error: Vec::new(),
    };
    assert!(matches!(
        outcome(Ok(killed)),
        Err(SandboxProfileError::Failed(text)) if text.contains("signal")
    ));
    // pkexec disappeared since the offer: back to the manual instructions.
    assert_eq!(
        outcome(Err(io::Error::from(io::ErrorKind::NotFound))),
        Err(SandboxProfileError::Unavailable)
    );
    assert_eq!(
        outcome(Err(io::Error::other("cannot start"))),
        Err(SandboxProfileError::Failed("cannot start".into()))
    );
}

#[test]
fn an_untrusted_pkexec_never_runs() {
    let result = install_with(
        b"text",
        &Cancellation::default(),
        || {
            Err(io::Error::other(
                "Untrusted system authorization executable",
            ))
        },
        |_, _, _| panic!("an untrusted pkexec must not run"),
    );
    assert_eq!(result, Err(SandboxProfileError::Unavailable));
}

#[test]
fn the_runner_delivers_the_input_and_keeps_the_start_of_the_error_output() {
    let (_temp, base) = base();
    let received = base.join("received");
    let mut command = Command::new(SHELL);
    command
        .arg("-c")
        .arg("cat >\"$0\"; printf 'first\\nsecond\\n' >&2; exit 3")
        .arg(&received);
    let finished = run(&mut command, b"profile", &Cancellation::default()).unwrap();
    assert_eq!(finished.status.code(), Some(3));
    assert_eq!(finished.error, b"first\nsecond\n");
    assert_eq!(fs::read(&received).unwrap(), b"profile");
    // A command that writes far more than a pipe holds neither blocks nor grows the text.
    let mut command = Command::new(SHELL);
    command.arg("-c").arg(
        "i=0; while [ $i -lt 3000 ]; do echo 0123456789012345678901234567890123456789 >&2; \
         i=$((i + 1)); done",
    );
    let finished = run(&mut command, b"", &Cancellation::default()).unwrap();
    assert_eq!(finished.status.code(), Some(0));
    assert_eq!(finished.error.len(), ERROR_BYTES);
    // One that ends without reading its input still reports its own status.
    let mut command = Command::new(SHELL);
    command.arg("-c").arg("exit 126");
    let finished = run(&mut command, &[b'x'; 256], &Cancellation::default()).unwrap();
    assert_eq!(finished.outcome(), Err(SandboxProfileError::Declined));
}

#[test]
fn cancellation_stops_a_waiting_authorization() {
    let cancel = Cancellation::default();
    let stop = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        stop.cancel();
    });
    let started = Instant::now();
    let mut command = Command::new(SHELL);
    command.arg("-c").arg("exec sleep 30");
    let error = run(&mut command, b"", &cancel).unwrap_err();
    canceller.join().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_command_that_ended_before_its_cancellation_reports_its_own_status() {
    // As an installation that pkexec already ran as root, which this process cannot
    // stop: its outcome is the command's, never "cancelled".
    for code in [0, 3] {
        let mut child = Command::new(SHELL)
            .arg("-c")
            .arg(format!("exit {code}"))
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        // Wait for the exit without reaping, as `wait` finds it at cancellation.
        let pid = child_pid(child.id());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ended(pid) {
            assert!(Instant::now() < deadline, "the command never ended");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(stop(&mut child).unwrap().code(), Some(code));
    }
    // A command still waiting is stopped and reported as cancelled.
    let mut child = Command::new(SHELL)
        .arg("-c")
        .arg("exec sleep 30")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    assert_eq!(
        stop(&mut child).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

fn child_pid(pid: u32) -> libc::pid_t {
    libc::pid_t::try_from(pid).unwrap()
}

/// Whether process `pid`, a child of this process, has exited (a zombie not yet reaped).
fn ended(pid: libc::pid_t) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            let state = stat.rsplit_once(") ")?.1.chars().next()?;
            Some(state == 'Z')
        })
        .unwrap_or(false)
}

/// The root command, unprivileged, with its profile folder in `folder` and `parser` as
/// the only `apparmor_parser` it can find.
fn unprivileged(folder: &Path, parser: &Path) -> String {
    let path_line = "PATH=/usr/sbin:/usr/bin:/sbin:/bin\n";
    assert_eq!(INSTALL.matches(path_line).count(), 1);
    assert!(INSTALL.contains("/etc/apparmor.d"));
    let script = INSTALL
        .replace(
            path_line,
            &format!("PATH={}:/usr/bin:/bin\n", parser.display()),
        )
        .replace("/etc/apparmor.d", &folder.display().to_string());
    assert!(
        !script.contains("/etc/") && !script.contains("sbin"),
        "{script}"
    );
    script
}

struct RootCommand {
    _temp: tempfile::TempDir,
    folder: PathBuf,
    log: PathBuf,
    script: String,
}

impl RootCommand {
    /// A fake `apparmor_parser` that logs its arguments and refuses a check (`-QTK`) when
    /// `refuse` is set.
    fn new(refuse: bool) -> Self {
        let (temp, base) = base();
        let folder = base.join("apparmor.d");
        let parser = base.join("bin");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&parser).unwrap();
        let log = base.join("log");
        let fake = parser.join("apparmor_parser");
        fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" >>'{}'\n{}exit 0\n",
                log.display(),
                if refuse {
                    "[ \"$1\" = -QTK ] && { echo 'parser: syntax error' >&2; exit 1; }\n"
                } else {
                    ""
                }
            ),
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let script = unprivileged(&folder, &parser);
        Self {
            _temp: temp,
            folder,
            log,
            script,
        }
    }

    fn run(&self, input: &[u8]) -> Finished {
        let mut command = Command::new(SHELL);
        command.arg("-c").arg(&self.script).env_clear();
        run(&mut command, input, &Cancellation::default()).unwrap()
    }

    fn target(&self) -> PathBuf {
        self.folder.join(PROFILE)
    }

    /// Every entry of the profile folder: a staged file never remains.
    fn entries(&self) -> Vec<String> {
        let mut entries: Vec<String> = fs::read_dir(&self.folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        entries.sort();
        entries
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

#[test]
fn the_root_command_checks_writes_and_loads_the_profile_atomically() {
    let root = RootCommand::new(false);
    let text = profile_text(&format!("/opt/me/{VIEWER}"));
    let finished = root.run(text.as_bytes());
    assert_eq!(finished.outcome(), Ok(()), "{finished:?}");
    assert_eq!(fs::read_to_string(root.target()).unwrap(), text);
    let mode = fs::metadata(root.target()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o644);
    assert_eq!(root.entries(), [PROFILE]);
    // The staged file is checked before it replaces the profile, which is then loaded.
    let log = root.log();
    let lines: Vec<&str> = log.lines().collect();
    let staged = format!("{}/.{PROFILE}.", root.folder.display());
    assert!(
        lines.len() == 2 && lines[0].starts_with(&format!("-QTK {staged}")),
        "{log}"
    );
    assert_eq!(lines[1], format!("-r {}", root.target().display()));
    // Installing again replaces the profile.
    let replaced = profile_text(&format!("/srv/me/{VIEWER}"));
    assert_eq!(root.run(replaced.as_bytes()).outcome(), Ok(()));
    assert_eq!(fs::read_to_string(root.target()).unwrap(), replaced);
    assert_eq!(root.entries(), [PROFILE]);
}

#[test]
fn a_refused_or_missing_profile_text_leaves_the_installed_profile() {
    let root = RootCommand::new(true);
    fs::write(root.target(), "installed").unwrap();
    let finished = root.run(profile_text(&format!("/opt/me/{VIEWER}")).as_bytes());
    let Err(SandboxProfileError::Failed(text)) = finished.outcome() else {
        panic!("a refused profile fails: {finished:?}");
    };
    assert!(
        text.contains("parser: syntax error") && text.contains("AppArmor refused the profile."),
        "{text}"
    );
    assert_eq!(fs::read_to_string(root.target()).unwrap(), "installed");
    assert_eq!(root.entries(), [PROFILE]);
    assert!(!root.log().contains("-r "), "nothing is loaded");
    let finished = root.run(b"");
    assert_eq!(
        finished.outcome(),
        Err(SandboxProfileError::Failed(
            "No profile text was received.".into()
        ))
    );
    assert_eq!(fs::read_to_string(root.target()).unwrap(), "installed");
    assert_eq!(root.entries(), [PROFILE]);
}

#[test]
fn the_root_command_never_writes_into_a_folder_in_the_profiles_place() {
    let root = RootCommand::new(false);
    fs::create_dir(root.target()).unwrap();
    let finished = root.run(profile_text(&format!("/opt/me/{VIEWER}")).as_bytes());
    assert!(
        matches!(finished.outcome(), Err(SandboxProfileError::Failed(text))
            if text.contains("Cannot replace")),
        "{finished:?}"
    );
    assert_eq!(fs::read_dir(root.target()).unwrap().count(), 0);
    assert_eq!(root.entries(), [PROFILE]);
}

#[test]
fn a_system_without_apparmor_parser_is_told_which_package_is_missing() {
    if ["/usr/bin/apparmor_parser", "/bin/apparmor_parser"]
        .iter()
        .any(|path| Path::new(path).exists())
    {
        eprintln!("apparmor_parser is in /usr/bin: its absence cannot be tested here");
        return;
    }
    let (_temp, base) = base();
    let folder = base.join("apparmor.d");
    let empty = base.join("bin");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(&empty).unwrap();
    let mut command = Command::new(SHELL);
    command
        .arg("-c")
        .arg(unprivileged(&folder, &empty))
        .env_clear();
    let finished = run(&mut command, b"text", &Cancellation::default()).unwrap();
    assert_eq!(
        finished.outcome(),
        Err(SandboxProfileError::Failed(
            "apparmor_parser is missing (package apparmor).".into()
        ))
    );
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
}

#[test]
fn the_runner_reaps_a_stopped_command_before_it_returns() {
    // A cancelled run must not leave a child behind: the pid no longer exists afterwards.
    let (_temp, base) = base();
    let pid_file = base.join("pid");
    let cancel = Cancellation::default();
    let stop = cancel.clone();
    let written = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (pid_file, written) = (pid_file.clone(), written.clone());
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let complete = || fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n'));
            while !complete() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            written.store(complete(), Ordering::Release);
            stop.cancel();
        })
    };
    let mut command = Command::new(SHELL);
    command
        .arg("-c")
        .arg("echo $$ >\"$0\"; exec sleep 30")
        .arg(&pid_file);
    assert!(run(&mut command, b"", &cancel).is_err());
    watcher.join().unwrap();
    assert!(written.load(Ordering::Acquire));
    let pid = fs::read_to_string(&pid_file).unwrap();
    assert!(
        !Path::new("/proc").join(pid.trim()).exists(),
        "the stopped command was reaped"
    );
}
