use super::super::{Snapshot, SnapshotPixels, encode, publish};
use super::*;
use midnight_core::{
    Cancellation, ForeignSetting, ForeignSettingKind, PathSystem, UserCommand, UserCommandId,
    UserCommandInvocation, UserCommandLaunch,
};
use std::fs::OpenOptions;

const MONTH: Duration = Duration::from_secs(30 * 24 * 60 * 60);

fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
/// Opens with write access too, as the native adapters' handles may set the time.
fn open(path: &Path) -> io::Result<Option<File>> {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn plain_file_modified(path: &Path) -> io::Result<Option<SystemTime>> {
    let metadata = fs::symlink_metadata(path)?;
    metadata.is_file().then(|| metadata.modified()).transpose()
}
fn remove(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}
fn name(index: usize) -> String {
    format!("{index:064x}.png")
}
fn stored(directory: &Path, name: &str, modified: SystemTime) -> PathBuf {
    let path = directory.join(name);
    File::create(&path).unwrap().set_modified(modified).unwrap();
    path
}
fn snapshot(seed: u8) -> Snapshot {
    let pixels = SnapshotPixels::new(1, 1, vec![seed, 7, 9, 255]).unwrap();
    encode(&pixels, &Cancellation::default()).unwrap()
}
fn command(id: &str, icon: &Path) -> UserCommand {
    UserCommand {
        output: Default::default(),
        id: UserCommandId::new(id).unwrap(),
        name: id.into(),
        enabled: true,
        toolbar_label: None,
        icon: Some(icon.to_owned()),
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: Vec::new(),
        },
    }
}
fn collect_now(directory: &Path, keep: &BTreeSet<String>) -> usize {
    collect_unreferenced(
        directory,
        || keep.clone(),
        SystemTime::now(),
        COLLECTION_GRACE,
        create,
        open,
        plain_file_modified,
        remove,
    )
    .unwrap()
}

#[test]
fn a_full_folder_that_no_setting_names_is_freed_and_the_next_import_succeeds() {
    let folder = tempfile::tempdir().unwrap();
    let old = SystemTime::now() - MONTH;
    for index in 0..1024 {
        stored(folder.path(), &name(index), old);
    }
    let cancel = Cancellation::default();
    let next = snapshot(1);
    let full = publish(folder.path(), &next, &cancel, create, open).unwrap_err();
    assert!(full.to_string().contains("is full"), "{full}");

    assert_eq!(collect_now(folder.path(), &BTreeSet::new()), 1024);
    let path = publish(folder.path(), &next, &cancel, create, open).unwrap();
    assert!(path.is_file());
}

#[test]
fn named_snapshots_stay_and_a_shared_one_goes_only_when_no_setting_names_it() {
    let folder = tempfile::tempdir().unwrap();
    let old = SystemTime::now() - MONTH;
    let shared = stored(folder.path(), &name(1), old);
    let single = stored(folder.path(), &name(2), old);
    let unnamed = stored(folder.path(), &name(3), old);
    let mut preferences = UserPreferences {
        user_commands: vec![
            command("first", &shared),
            command("second", &shared),
            command("third", &single),
        ]
        .into(),
        ..Default::default()
    };
    assert_eq!(
        collect_now(folder.path(), &referenced_names(&preferences)),
        1
    );
    assert!(shared.is_file() && single.is_file());
    assert!(!unnamed.exists());

    // The first command drops the shared picture: the second still names it.
    preferences.user_commands = vec![command("second", &shared)].into();
    assert_eq!(
        collect_now(folder.path(), &referenced_names(&preferences)),
        1
    );
    assert!(shared.is_file());
    assert!(!single.exists());

    preferences.user_commands = Vec::new().into();
    assert_eq!(
        collect_now(folder.path(), &referenced_names(&preferences)),
        1
    );
    assert!(!shared.exists());
}

#[test]
fn a_snapshot_held_by_an_unsaved_draft_survives_a_collection_meanwhile() {
    let folder = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    // This process issued it to a picker: an open editor's draft may hold it.
    let drafted = publish(folder.path(), &snapshot(2), &cancel, create, open).unwrap();
    let old = SystemTime::now() - MONTH;
    File::options()
        .write(true)
        .open(&drafted)
        .unwrap()
        .set_modified(old)
        .unwrap();
    // Recent and unnamed: another instance may have just picked it for its draft.
    let recent = stored(folder.path(), &name(4), SystemTime::now());
    let abandoned = stored(folder.path(), &name(5), old);

    let removed = collect(
        folder.path(),
        &BTreeSet::new(),
        create,
        open,
        plain_file_modified,
        remove,
    )
    .unwrap();
    assert_eq!(removed, 1);
    assert!(drafted.is_file(), "issued in this process");
    assert!(recent.is_file(), "within the grace period");
    assert!(!abandoned.exists());
}

#[test]
fn reusing_an_old_snapshot_restarts_its_grace_period() {
    let folder = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let picked = snapshot(3);
    let path = publish(folder.path(), &picked, &cancel, create, open).unwrap();
    let old = SystemTime::now() - MONTH;
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert_eq!(
        publish(folder.path(), &picked, &cancel, create, open).unwrap(),
        path
    );
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    assert!(SystemTime::now().duration_since(modified).unwrap() < REUSE_WITHOUT_REFRESH);
    // Even a collection that knows nothing of this process keeps it now.
    assert_eq!(collect_now(folder.path(), &BTreeSet::new()), 0);
    assert!(path.is_file());
}

#[test]
fn only_old_snapshot_files_are_removed_never_other_entries() {
    let folder = tempfile::tempdir().unwrap();
    let old = SystemTime::now() - MONTH;
    let others = [
        ".snapshot.lock",
        ".midnight-preferences-7-1.tmp",
        "notes.txt",
        "ABCDEF.png",
        &format!("{:064X}.png", 0xab),
        &format!("{:063x}.png", 6),
        &format!("{:064x}.PNG", 6),
    ]
    .map(|other| stored(folder.path(), other, old));
    let directory = folder.path().join(name(7));
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("inside"), b"kept").unwrap();

    assert_eq!(collect_now(folder.path(), &BTreeSet::new()), 0);
    for other in others {
        assert!(other.is_file(), "{}", other.display());
    }
    assert_eq!(fs::read(directory.join("inside")).unwrap(), b"kept");
}

#[test]
fn a_busy_lock_skips_the_collection() {
    let folder = tempfile::tempdir().unwrap();
    let abandoned = stored(folder.path(), &name(8), SystemTime::now() - MONTH);
    let lock = create(&folder.path().join(".snapshot.lock")).unwrap();
    lock.lock().unwrap();
    let busy = collect_unreferenced(
        folder.path(),
        BTreeSet::new,
        SystemTime::now(),
        COLLECTION_GRACE,
        create,
        open,
        plain_file_modified,
        remove,
    )
    .unwrap_err();
    assert_eq!(busy.kind(), io::ErrorKind::WouldBlock);
    assert!(abandoned.is_file());
    // Unlocked explicitly: a child process another test starts may share the file.
    lock.unlock().unwrap();
}

#[test]
fn names_come_from_command_icons_and_records_kept_for_another_system() {
    let folder = tempfile::tempdir().unwrap();
    let mut other = command("other", &folder.path().join("picture.png"));
    other.icon = Some(folder.path().join("ABCDEF.png"));
    let foreign_name = name(9);
    let preferences = UserPreferences {
        user_commands: vec![
            command("managed", &folder.path().join(name(10))),
            other,
            command("plain", &folder.path().join("picture.png")),
        ]
        .into(),
        foreign_settings: vec![ForeignSetting {
            kind: ForeignSettingKind::UserCommand {
                position: 3,
                id: UserCommandId::new("foreign").unwrap(),
                name: "Foreign".into(),
            },
            system: PathSystem::Unix,
            location: "/elsewhere".into(),
            stored: format!(r#"{{"icon":"/home/user/.config/icons/{foreign_name}"}}"#).into(),
        }]
        .into(),
        ..Default::default()
    };
    assert_eq!(
        referenced_names(&preferences),
        BTreeSet::from([name(10), foreign_name])
    );
}
