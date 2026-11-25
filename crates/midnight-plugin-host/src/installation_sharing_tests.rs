//! A shared installation loads its plugins and is reported; securing it or copying it into
//! the profile makes it private. Fixtures change permissions only inside their own folders.
use super::*;

/// A private folder for fixtures: below `HOST_TEST_INSTALL_ROOT` on Windows (whose ancestors
/// must be private), a new mode-700 temporary folder on Linux.
fn base() -> tempfile::TempDir {
    #[cfg(windows)]
    {
        let base =
            std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("protected HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        trust::protect_fixture(root.path()).unwrap();
        root
    }
    #[cfg(not(windows))]
    tempfile::tempdir().unwrap()
}

/// A small release-like installation: the executable, a viewer package with a nested folder,
/// and notices.
fn install(folder: &Path) {
    fs::create_dir(folder).unwrap();
    fs::write(folder.join("midnight-explorer.exe"), b"application").unwrap();
    fs::write(folder.join("LICENSE.txt"), b"licence").unwrap();
    let package = folder.join("viewers").join("basic");
    fs::create_dir_all(package.join("notices")).unwrap();
    fs::write(package.join("fixture.dll"), vec![7_u8; 3 * 1024 * 1024 + 5]).unwrap();
    fs::write(package.join("notices").join("NOTICE.txt"), b"notice").unwrap();
    fs::write(
        package.join("plugin.toml"),
        format!(
            "schema_version=1\nid=\"viewer.basic\"\nkind=\"viewer\"\nversion=\"0.1.0\"\n\
             entry_library=\"fixture.dll\"\ntarget=\"{}\"\nabi_major=1\nminimum_host_minor=0\n\
             models=[\"text-page.v1\"]\n",
            crate::host_target()
        ),
    )
    .unwrap();
}

#[test]
fn copy_verifies_every_file_and_asks_before_replacing() {
    let base = base();
    let source = base.path().join("extracted");
    install(&source);
    let profile = base.path().join("profile").join("Programs");
    let destination = profile.join("Midnight Explorer");
    let copied = copy_installation(&source, &destination, false).unwrap();
    assert_eq!(copied.files, 5);
    assert_eq!(
        copied.bytes,
        11 + 7 + 3 * 1024 * 1024 + 5 + 6 + {
            fs::metadata(source.join("viewers/basic/plugin.toml"))
                .unwrap()
                .len()
        }
    );
    assert!(copied.sharing.is_private(), "{:?}", copied.sharing);
    assert_eq!(
        fs::read(copied.folder.join("viewers/basic/fixture.dll")).unwrap(),
        fs::read(source.join("viewers/basic/fixture.dll")).unwrap()
    );
    // The original stays, and an existing copy is replaced only when asked.
    assert!(source.join("midnight-explorer.exe").is_file());
    assert_eq!(
        copy_installation(&source, &destination, false).unwrap_err(),
        CopyInstallationError::DestinationExists(destination.clone())
    );
    fs::write(source.join("LICENSE.txt"), b"updated licence").unwrap();
    let replaced = copy_installation(&source, &destination, true).unwrap();
    assert_eq!(
        fs::read(replaced.folder.join("LICENSE.txt")).unwrap(),
        b"updated licence"
    );
    let siblings: Vec<_> = fs::read_dir(&profile)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        siblings,
        ["Midnight Explorer"],
        "no staging or previous copy remains"
    );
    // Copying into the installation itself, or over it, is refused.
    for target in [source.join("nested"), source.clone()] {
        assert!(matches!(
            copy_installation(&source, &target, true),
            Err(CopyInstallationError::Failed(_))
        ));
    }
    assert!(source.join("midnight-explorer.exe").is_file());
}

#[test]
fn profile_installation_folder_is_absolute_and_named_for_the_platform() {
    let Some(folder) = profile_installation_folder() else {
        return;
    };
    assert!(folder.is_absolute());
    if cfg!(windows) {
        assert!(folder.ends_with("Programs/Midnight Explorer"), "{folder:?}");
    } else {
        assert!(folder.ends_with("midnight-explorer/app"), "{folder:?}");
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().mode() & 0o7777
    }

    #[test]
    fn securing_removes_group_and_other_write_bits_and_copies_keep_execute_bits() {
        let base = base();
        let folder = base.path().join("app");
        install(&folder);
        let executable = folder.join("midnight-explorer.exe");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o777)).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o777)).unwrap();
        let notice = folder.join("viewers/basic/notices/NOTICE.txt");
        fs::set_permissions(&notice, fs::Permissions::from_mode(0o666)).unwrap();
        let sharing = installation_sharing(&folder).unwrap();
        assert_eq!(sharing.shared_count, 3, "{sharing:?}");
        assert!(crate::validate_plugin_path(&executable).is_ok());
        let copied = copy_installation(&folder, &base.path().join("copy"), false).unwrap();
        assert_eq!(mode(&copied.folder.join("midnight-explorer.exe")), 0o755);
        assert!(copied.sharing.is_private(), "{:?}", copied.sharing);
        let secured = secure_installation(&folder).unwrap();
        assert!(secured.sharing.is_private(), "{secured:?}");
        assert!(secured.failures.is_empty());
        assert_eq!(mode(&folder), 0o755);
        assert_eq!(mode(&executable), 0o755);
        assert_eq!(mode(&notice), 0o644);
        let link = folder.join("link");
        std::os::unix::fs::symlink(&notice, &link).unwrap();
        assert_eq!(installation_sharing(&folder).unwrap().refused.len(), 1);
        assert!(copy_installation(&folder, &base.path().join("refused"), false).is_err());
        assert!(!base.path().join("refused").exists());
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::trust::{fixture_acl, fixture_explicit_acl, repair::dacl_sddl, user_sid_string};

    /// Modify (read, write, execute, delete), as Windows grants Authenticated Users below a
    /// drive root.
    const MODIFY: &str = "0x1301bf";
    /// The restricted workers' AppContainer, which installations grant read and execute.
    const WORKER: &str =
        "S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998";

    /// A folder like a drive root: private itself, but its subfolders inherit Modify for
    /// Authenticated Users.
    fn drive_root_like(path: &Path) {
        let user = user_sid_string().unwrap();
        fs::create_dir(path).unwrap();
        fixture_acl(
            path,
            &format!(
                "D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)\
                 (A;OICIIO;{MODIFY};;;AU)"
            ),
        )
        .unwrap();
    }

    /// A folder created below a drive root: it and everything below it inherit Modify for
    /// Authenticated Users.
    fn shared_folder(path: &Path) {
        let user = user_sid_string().unwrap();
        fs::create_dir(path).unwrap();
        fixture_acl(
            path,
            &format!(
                "D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)\
                 (A;OICI;{MODIFY};;;AU)"
            ),
        )
        .unwrap();
    }

    fn grants_authenticated_users(path: &Path) -> bool {
        dacl_sddl(path).unwrap().contains(";;;AU)")
    }

    #[test]
    fn inherited_authenticated_users_modify_installation_loads_plugins_and_is_reported() {
        let base = base();
        let parent = base.path().join("drive-root");
        drive_root_like(&parent);
        let folder = parent.join("me");
        install(&folder);
        assert!(grants_authenticated_users(
            &folder.join("viewers/basic/fixture.dll")
        ));
        let runtime = crate::Runtime::discover(&folder).unwrap();
        assert_eq!(runtime.catalog().len(), 1, "{:?}", runtime.diagnostics());
        let package = folder.join("viewers/basic").canonicalize().unwrap();
        crate::validate_plugin_package(&package).unwrap();
        let sharing = installation_sharing(&folder).unwrap();
        assert_eq!(sharing.shared.first(), Some(&folder));
        assert_eq!(sharing.shared_count, 9, "{sharing:?}");
        assert_eq!(sharing.shared_ancestor, None);
        assert!(sharing.refused.is_empty() && sharing.complete);
        // A folder redirected out of the installation stays refused.
        let outside = base.path().join("outside");
        fs::create_dir(&outside).unwrap();
        crate::tests_windows::junction(&package.join("lib"), &outside);
        let error = crate::validate_plugin_package(&package).unwrap_err();
        assert!(error.0.contains("redirect"), "{error}");
        assert_eq!(installation_sharing(&folder).unwrap().refused.len(), 1);
        assert!(copy_installation(&folder, &base.path().join("copy"), false).is_err());
        assert!(!base.path().join("copy").exists());
        fs::remove_dir(package.join("lib")).unwrap();
    }

    #[test]
    fn securing_protects_a_folder_whose_parent_passes_on_shared_access() {
        let base = base();
        let parent = base.path().join("drive-root");
        drive_root_like(&parent);
        let folder = parent.join("me");
        install(&folder);
        let library = folder.join("viewers/basic/fixture.dll");
        fixture_explicit_acl(&library, &format!("D:(A;;0x1200a9;;;{WORKER})")).unwrap();
        let secured = secure_installation(&folder).unwrap();
        assert!(secured.sharing.is_private(), "{secured:?}");
        assert!(secured.failures.is_empty(), "{secured:?}");
        assert!(dacl_sddl(&folder).unwrap().starts_with("D:P"));
        for path in [&folder, &library, &folder.join("midnight-explorer.exe")] {
            assert!(!grants_authenticated_users(path), "{}", path.display());
        }
        // The workers' explicit read and execute grant stays, and the library now inherits.
        let library_dacl = dacl_sddl(&library).unwrap();
        assert!(library_dacl.contains(WORKER), "{library_dacl}");
        assert!(!library_dacl.starts_with("D:P"), "{library_dacl}");
        assert!(crate::trust::check_private(&library).is_ok());
    }

    #[test]
    fn securing_a_moved_folder_reinherits_from_its_private_parent() {
        let base = base();
        let shared = base.path().join("tools");
        shared_folder(&shared);
        install(&shared.join("me"));
        // A same-volume move keeps the inherited entries of the former location.
        let profile = base.path().join("profile");
        fs::create_dir(&profile).unwrap();
        let folder = profile.join("me");
        fs::rename(shared.join("me"), &folder).unwrap();
        assert!(grants_authenticated_users(
            &folder.join("midnight-explorer.exe")
        ));
        let sharing = installation_sharing(&folder).unwrap();
        assert!(!sharing.is_private());
        assert_eq!(sharing.shared_ancestor, None, "the new parent is private");
        let secured = secure_installation(&folder).unwrap();
        assert!(secured.sharing.is_private(), "{secured:?}");
        assert!(secured.failures.is_empty(), "{secured:?}");
        // Inheritance from the private profile folder, not a protected copy of it.
        assert!(!dacl_sddl(&folder).unwrap().starts_with("D:P"));
        assert!(!grants_authenticated_users(
            &folder.join("viewers/basic/fixture.dll")
        ));
        assert!(crate::trust::check_private(&folder.join("midnight-explorer.exe")).is_ok());
    }

    #[test]
    fn securing_reports_what_other_accounts_can_still_change() {
        let base = base();
        let tools = base.path().join("tools");
        shared_folder(&tools);
        let folder = tools.join("me");
        install(&folder);
        // An object whose permissions the user cannot change.
        let user = user_sid_string().unwrap();
        let locked = folder.join("LICENSE.txt");
        fixture_acl(
            &locked,
            &format!("D:P(D;;WD;;;{user})(A;;FA;;;WD)(A;;0x1;;;OW)"),
        )
        .unwrap();
        let secured = secure_installation(&folder).unwrap();
        // The folder containing the installation is never changed and is reported.
        assert_eq!(
            secured.sharing.shared_ancestor.as_deref(),
            Some(tools.as_path())
        );
        assert_eq!(secured.sharing.shared, [locked], "{secured:?}");
        assert!(
            secured
                .failures
                .iter()
                .any(|failure| failure.contains("LICENSE.txt")),
            "{secured:?}"
        );
        assert!(grants_authenticated_users(&tools));
        assert!(secure_installation(&base.path().join("missing")).is_err());
    }

    #[test]
    fn copying_a_shared_installation_gives_a_private_copy_with_worker_grants() {
        let base = base();
        let parent = base.path().join("drive-root");
        drive_root_like(&parent);
        let source = parent.join("me");
        install(&source);
        let library = source.join("viewers/basic/fixture.dll");
        fixture_explicit_acl(&library, &format!("D:(A;;0x1200a9;;;{WORKER})")).unwrap();
        let profile = base.path().join("profile");
        fs::create_dir(&profile).unwrap();
        let copied = copy_installation(&source, &profile.join("Midnight Explorer"), false).unwrap();
        assert!(copied.sharing.is_private(), "{:?}", copied.sharing);
        assert!(!grants_authenticated_users(
            &copied.folder.join("LICENSE.txt")
        ));
        let copy = copied.folder.join("viewers/basic/fixture.dll");
        assert!(dacl_sddl(&copy).unwrap().contains(WORKER));
        assert!(
            grants_authenticated_users(&source.join("LICENSE.txt")),
            "source unchanged"
        );
    }

    /// A folder of the user's outside the installation whose parent passes on Modify for
    /// Authenticated Users, with a protected subfolder that only the user and one other account
    /// (Backup Operators stands in for a colleague) may change.
    fn victim(base: &Path) -> (PathBuf, PathBuf, String) {
        let projects = base.join("projects");
        shared_folder(&projects);
        let secret = projects.join("secret");
        fs::create_dir(&secret).unwrap();
        let user = user_sid_string().unwrap();
        fixture_acl(
            &secret,
            &format!("D:P(A;OICI;FA;;;{user})(A;OICI;{MODIFY};;;BO)"),
        )
        .unwrap();
        fs::write(secret.join("notes.txt"), b"notes").unwrap();
        let before = dacl_sddl(&secret).unwrap();
        (projects, secret, before)
    }

    #[test]
    fn securing_never_changes_a_junction_target_outside_the_installation() {
        let base = base();
        let parent = base.path().join("drive-root");
        drive_root_like(&parent);
        let folder = parent.join("me");
        install(&folder);
        let (projects, secret, before) = victim(base.path());
        // Planted before the repair: the junction itself is reported, its target untouched.
        crate::tests_windows::junction(&folder.join("link"), &projects);
        let secured = secure_installation(&folder).unwrap();
        assert!(!secured.sharing.is_private(), "{secured:?}");
        assert!(
            secured
                .sharing
                .refused
                .iter()
                .any(|reason| reason.contains("link")),
            "{secured:?}"
        );
        assert_eq!(dacl_sddl(&secret).unwrap(), before);
        assert!(grants_authenticated_users(&projects));
        // Swapped in during the repair: a folder the walk has already listed becomes a
        // junction, so the next path below it leads out of the installation. Securing that
        // path must refuse, not unprotect the target (which would then inherit Modify for
        // Authenticated Users from its parent).
        let canonical = folder.canonicalize().unwrap();
        let error = crate::trust::repair::secure_entry(
            &folder.join("link").join("secret"),
            &canonical.join("link").join("secret"),
        )
        .unwrap_err();
        assert!(error.0.contains("leads out of the installation"), "{error}");
        assert_eq!(dacl_sddl(&secret).unwrap(), before);
        assert!(!grants_authenticated_users(&secret.join("notes.txt")));
        // The junction itself is refused as well, never changed through its target.
        let error =
            crate::trust::repair::secure_entry(&folder.join("link"), &canonical.join("link"))
                .unwrap_err();
        assert!(error.0.contains("is a link"), "{error}");
        fs::remove_dir(folder.join("link")).unwrap();
        assert_eq!(dacl_sddl(&secret).unwrap(), before);
    }

    #[test]
    fn securing_an_entry_by_handle_keeps_explicit_grants_and_reinherits() {
        let base = base();
        let parent = base.path().join("drive-root");
        drive_root_like(&parent);
        let folder = parent.join("me");
        install(&folder);
        // A protected library granting other accounts Modify and the workers read and execute.
        let user = user_sid_string().unwrap();
        let library = folder.join("viewers/basic/fixture.dll");
        fixture_acl(
            &library,
            &format!("D:P(A;;FA;;;{user})(A;;{MODIFY};;;AU)(A;;0x1200a9;;;{WORKER})"),
        )
        .unwrap();
        let canonical = folder.canonicalize().unwrap();
        crate::trust::repair::secure_entry(
            &library,
            &canonical.join("viewers").join("basic").join("fixture.dll"),
        )
        .unwrap();
        let dacl = dacl_sddl(&library).unwrap();
        assert!(!dacl.starts_with("D:P"), "{dacl}");
        assert!(dacl.contains(WORKER), "{dacl}");
        assert!(!dacl.contains(&format!("(A;;{MODIFY};;;AU)")), "{dacl}");
        // It inherits its folder's entries again (here still Modify for Authenticated Users,
        // which the folder-wide change removes).
        assert!(dacl.contains(";ID;"), "{dacl}");
    }

    /// The workers' read and execute entry, as the hosts and the build scripts add it.
    fn worker_entry() -> String {
        format!("(A;;0x1200a9;;;{WORKER})")
    }

    #[test]
    fn the_workers_grant_is_added_only_where_the_installation_is_private() {
        let base = base();
        let parent = base.path().join("drive-root");
        drive_root_like(&parent);
        let shared = parent.join("me");
        install(&shared);
        let shared = shared.canonicalize().unwrap();
        // A helper and its library extracted from a ZIP: both inherit only their folder's
        // permissions, which grant AppContainers nothing.
        let worker = shared.join("viewers/basic/worker.exe");
        let library = shared.join("viewers/basic/fixture.dll");
        let files = || vec![library.clone()];
        let before = dacl_sddl(&library).unwrap();
        assert!(!before.contains(WORKER), "{before}");
        // Other accounts can change this installation: nothing is changed.
        let error = crate::prepare_restricted_worker(&worker, files).unwrap_err();
        assert!(error.to_string().contains("not private"), "{error}");
        assert_eq!(dacl_sddl(&library).unwrap(), before);
        // A private copy in the profile gets the grant at its first launch; the shared source
        // keeps its permissions.
        let profile = base.path().join("profile");
        fs::create_dir(&profile).unwrap();
        let copied = copy_installation(&shared, &profile.join("Midnight Explorer"), false).unwrap();
        assert!(copied.sharing.is_private(), "{:?}", copied.sharing);
        let copy = copied.folder.join("viewers/basic/fixture.dll");
        assert!(!dacl_sddl(&copy).unwrap().contains(WORKER));
        crate::prepare_restricted_worker(&copied.folder.join("viewers/basic/worker.exe"), || {
            vec![copy.clone()]
        })
        .unwrap();
        let granted = dacl_sddl(&copy).unwrap();
        assert_eq!(granted.matches(&worker_entry()).count(), 1, "{granted}");
        assert_eq!(dacl_sddl(&library).unwrap(), before);
        // Securing the shared installation lets its next launch add the grant (a refusal is
        // not kept for the run), and the grant keeps it private.
        let secured = secure_installation(&shared).unwrap();
        assert!(secured.sharing.is_private(), "{secured:?}");
        crate::prepare_restricted_worker(&worker, files).unwrap();
        let granted = dacl_sddl(&library).unwrap();
        assert_eq!(granted.matches(&worker_entry()).count(), 1, "{granted}");
        assert!(installation_sharing(&shared).unwrap().is_private());
        // Securing again keeps the grant.
        secure_installation(&shared).unwrap();
        assert!(dacl_sddl(&library).unwrap().contains(&worker_entry()));
    }
}
