#[path = "../src/file_targets.rs"]
mod file_targets;
#[path = "../src/user_command_prepare.rs"]
mod prepare;
use midnight_core::*;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug)]
struct Sources {
    directory: PathBuf,
    selected: Arc<[Entry]>,
    focus: Option<Entry>,
    valid: Arc<AtomicBool>,
}
impl CapturedUserCommandSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(1)
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<ResolvedUserCommandSources, FsError> {
        self.revalidate(cancel)?;
        Ok(ResolvedUserCommandSources {
            directory: self.directory.clone(),
            selected: self.selected.clone(),
            focus: self.focus.clone(),
        })
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if cancel.is_cancelled() || !self.valid.load(Ordering::Acquire) {
            Err(FsError::from_io(
                "capture",
                &self.directory,
                std::io::Error::other("revoked"),
            ))
        } else {
            Ok(())
        }
    }
}
fn request(
    directory: &std::path::Path,
    selected: &[(&str, EntryKind)],
    focus: Option<&str>,
    launch: UserCommandLaunch,
    arguments: &[&str],
) -> (UserCommandRequest, Arc<AtomicBool>) {
    let command = UserCommand {
        id: UserCommandId::new("tree-command").unwrap(),
        name: "Tree command".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: arguments.iter().map(|arg| (*arg).into()).collect(),
        },
    };
    let valid = Arc::new(AtomicBool::new(true));
    let source = Arc::new(Sources {
        directory: directory.to_owned(),
        valid: valid.clone(),
        selected: selected
            .iter()
            .enumerate()
            .map(|(index, (name, kind))| {
                Entry::new(EntryId(index as u64 + 1), directory.join(name), *kind)
            })
            .collect::<Vec<_>>()
            .into(),
        focus: focus.map(|name| Entry::new(EntryId(10_000), directory.join(name), EntryKind::File)),
    });
    (
        UserCommandRequest {
            id: UserCommandRequestId(1),
            command: command.id.clone(),
            configuration: Arc::new(vec![command]),
            captured: Some(source),
            context: UserCommandContext {
                active: Some(directory.to_owned()),
                other: None,
                files: None,
                focus: None,
            },
        },
        valid,
    )
}
fn arguments(prepared: &prepare::Prepared, index: usize) -> &[std::ffi::OsString] {
    let UserCommandProcessInvocation::Program { arguments, .. } =
        &prepared.processes[index].invocation
    else {
        panic!("program")
    };
    arguments
}

#[test]
fn tree_user_command_once_and_per_selected_preserve_distinct_focus_and_native_order() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["focused.txt", "one & $x.txt", "two.txt"] {
        fs::write(directory.path().join(name), b"bytes").unwrap();
    }
    let service = midnight_platform::LocalFileSystem::new(None);
    let selected = [
        ("two.txt", EntryKind::File),
        ("one & $x.txt", EntryKind::File),
    ];
    let (request, _) = request(
        directory.path(),
        &selected,
        Some("focused.txt"),
        UserCommandLaunch::Single,
        &["${file.name}", "${selected.paths}"],
    );
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(prepared.processes.len(), 1);
    assert_eq!(
        arguments(&prepared, 0),
        [
            std::ffi::OsString::from("focused.txt"),
            directory.path().join("two.txt").into_os_string(),
            directory.path().join("one & $x.txt").into_os_string()
        ]
    );
    drop(prepared);
    let mut request = request;
    Arc::make_mut(&mut request.configuration)[0].launch = UserCommandLaunch::PerItem;
    Arc::make_mut(&mut request.configuration)[0].invocation = UserCommandInvocation::Program {
        executable: std::env::current_exe().unwrap(),
        arguments: vec!["${file.name}".into()],
    };
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(prepared.processes.len(), 2);
    assert_eq!(arguments(&prepared, 0), ["two.txt"]);
    assert_eq!(arguments(&prepared, 1), ["one & $x.txt"]);
}

#[test]
fn tree_user_command_literal_once_needs_no_file_and_capture_cannot_change_cwd_authority() {
    let directory = tempfile::tempdir().unwrap();
    let (mut request, _) = request(
        directory.path(),
        &[],
        None,
        UserCommandLaunch::Single,
        &["literal"],
    );
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare_preview(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(arguments(&prepared, 0), ["literal"]);
    let preview = prepared
        .into_preview(UserCommandRetention::new(()))
        .unwrap();
    assert_eq!(preview.processes.len(), 1);
    drop(preview);
    request.context.active = Some(directory.path().join("different"));
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
}

#[test]
fn tree_user_command_selected_kind_and_late_provider_revocation_fail_before_launch() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("was-file")).unwrap();
    let (request, _) = request(
        directory.path(),
        &[("was-file", EntryKind::File)],
        None,
        UserCommandLaunch::PerItem,
        &["${file.path}"],
    );
    let service = midnight_platform::LocalFileSystem::new(None);
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
    let (request, valid) =
        self::request(directory.path(), &[], None, UserCommandLaunch::Single, &[]);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    valid.store(false, Ordering::Release);
    assert!(
        prepared
            .revalidate_for(0, &Cancellation::default())
            .is_err()
    );
}

#[test]
fn tree_user_command_rejects_mixed_flat_authority_and_unmarked_per_item_uses_focus() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("focus"), b"data").unwrap();
    let (mut request, _) = request(
        directory.path(),
        &[],
        Some("focus"),
        UserCommandLaunch::PerItem,
        &["${file.name}"],
    );
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(arguments(&prepared, 0), ["focus"]);
    request.context.focus = Some(Entry::new(
        EntryId(4),
        directory.path().join("injected"),
        EntryKind::File,
    ));
    assert!(request.validate().is_err());
}

#[derive(Debug)]
struct Counted {
    inner: Arc<dyn CapturedUserCommandSources>,
    full: std::sync::atomic::AtomicUsize,
    scopes: std::sync::Mutex<Vec<Vec<EntryId>>>,
}
impl CapturedUserCommandSources for Counted {
    fn stamp(&self) -> TreeViewStamp {
        self.inner.stamp()
    }
    fn request(&self) -> TreeRequestId {
        self.inner.request()
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<ResolvedUserCommandSources, FsError> {
        self.inner.resolve(cancel)
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.full.fetch_add(1, Ordering::SeqCst);
        self.inner.revalidate(cancel)
    }
    fn revalidate_for(&self, entries: &[EntryId], cancel: &Cancellation) -> Result<(), FsError> {
        self.scopes.lock().unwrap().push(entries.to_vec());
        self.inner.revalidate_for(entries, cancel)
    }
}
#[test]
fn tree_user_command_per_item_revalidates_only_its_immutable_native_scope() {
    let directory = tempfile::tempdir().unwrap();
    let names = (0..256)
        .map(|index| format!("file-{index}"))
        .collect::<Vec<_>>();
    for name in &names {
        fs::write(directory.path().join(name), b"data").unwrap();
    }
    fs::write(directory.path().join("unmarked-focus"), b"data").unwrap();
    let selected = names
        .iter()
        .map(|name| (name.as_str(), EntryKind::File))
        .collect::<Vec<_>>();
    let (mut request, _) = request(
        directory.path(),
        &selected,
        Some("unmarked-focus"),
        UserCommandLaunch::PerItem,
        &["${file.path}"],
    );
    let counted = Arc::new(Counted {
        inner: request.captured.take().unwrap(),
        full: Default::default(),
        scopes: Default::default(),
    });
    request.captured = Some(counted.clone());
    let native = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &native, &Cancellation::default()).unwrap();
    let before = counted.full.load(Ordering::SeqCst);
    assert!(
        before > 0,
        "initial preparation must validate the whole capture"
    );
    for index in 0..256 {
        prepared
            .revalidate_for(index, &Cancellation::default())
            .unwrap();
    }
    assert_eq!(
        counted.full.load(Ordering::SeqCst),
        before,
        "no complete selection rescan per process"
    );
    assert_eq!(
        *counted.scopes.lock().unwrap(),
        (1..=256).map(|id| vec![EntryId(id)]).collect::<Vec<_>>()
    );
    drop(prepared);
    Arc::make_mut(&mut request.configuration)[0].launch = UserCommandLaunch::Single;
    let prepared = prepare::prepare(&request, &native, &Cancellation::default()).unwrap();
    prepared
        .revalidate_for(0, &Cancellation::default())
        .unwrap();
    assert_eq!(
        counted.scopes.lock().unwrap().last().unwrap(),
        &[EntryId(10_000)],
        "Once validates its unmarked focus"
    );
}

fn stale_metadata(request: &mut UserCommandRequest, valid: Arc<AtomicBool>) {
    let resolved = request
        .captured
        .as_ref()
        .unwrap()
        .resolve(&Cancellation::default())
        .unwrap();
    let mut selected = resolved.selected.to_vec();
    selected[0].metadata.modified = Some(std::time::UNIX_EPOCH);
    selected[0].metadata.size = Some(999);
    request.captured = Some(Arc::new(Sources {
        directory: resolved.directory,
        selected: selected.into(),
        focus: resolved.focus,
        valid,
    }));
}
#[test]
fn tree_user_command_directory_display_metadata_is_not_identity_but_pinned_changes_are_rejected() {
    for replace in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let selected = directory.path().join("directory");
        fs::create_dir(&selected).unwrap();
        let (mut request, valid) = request(
            directory.path(),
            &[("directory", EntryKind::Directory)],
            None,
            UserCommandLaunch::PerItem,
            &["${file.path}"],
        );
        stale_metadata(&mut request, valid);
        let service = midnight_platform::LocalFileSystem::new(None);
        let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
        prepared
            .revalidate_for(0, &Cancellation::default())
            .unwrap();
        if replace {
            fs::rename(&selected, directory.path().join("old-directory")).unwrap();
            fs::create_dir(&selected).unwrap();
        } else {
            // Directory child insertion can leave Windows timestamps unchanged
            // temporarily. Force and read back a real revision change instead.
            let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(86_400);
            let cancel = Cancellation::default();
            let mutation = MutationService::prepare(&service, &selected, &cancel).unwrap();
            let outcome = mutation.edit_metadata(
                &MetadataPatch {
                    modified: TimeEdit::Set(modified),
                    ..Default::default()
                },
                None,
                &cancel,
            );
            assert!(outcome.error.is_none(), "{outcome:?}");
            assert!(outcome.changes.modified);
            assert_eq!(
                service.entry_metadata(&selected).unwrap().modified,
                Some(modified)
            );
        }
        assert!(
            prepared
                .revalidate_for(0, &Cancellation::default())
                .is_err(),
            "after-pin change must fail"
        );
    }
}
#[test]
fn tree_user_command_regular_file_stale_display_metadata_is_still_rejected() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("file"), b"bytes").unwrap();
    let (mut request, valid) = request(
        directory.path(),
        &[("file", EntryKind::File)],
        None,
        UserCommandLaunch::PerItem,
        &["${file.path}"],
    );
    stale_metadata(&mut request, valid);
    assert!(
        prepare::prepare(
            &request,
            &midnight_platform::LocalFileSystem::new(None),
            &Cancellation::default()
        )
        .is_err()
    );
}
