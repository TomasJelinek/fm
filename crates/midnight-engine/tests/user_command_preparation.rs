#[path = "../src/file_targets.rs"]
mod file_targets;
#[path = "../src/user_command_prepare.rs"]
mod prepare;

use midnight_core::*;
use std::{fs, path::Path, sync::Arc};

fn configuration(
    executable: &Path,
    arguments: &[&str],
    launch: UserCommandLaunch,
) -> Arc<Vec<UserCommand>> {
    Arc::new(vec![UserCommand {
        id: UserCommandId::new("tool").unwrap(),
        name: "Tool".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch,
        invocation: UserCommandInvocation::Program {
            executable: executable.to_owned(),
            arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
        },
    }])
}
fn capture(directory: &Path, selected: &[&str], focus: &str) -> UserCommandContext {
    let entries = selected
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(EntryId(index as u64), directory.join(name), EntryKind::File)
        })
        .collect::<Vec<_>>();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let bits = if selected.is_empty() {
        vec![]
    } else {
        vec![(1u64 << selected.len()) - 1]
    };
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap());
    UserCommandContext {
        active: Some(directory.to_owned()),
        other: Some(directory.to_owned()),
        files: Some(SelectedFiles {
            selection,
            view,
            fallback: Some(directory.join(focus)),
        }),
        focus: Some(Entry::new(
            EntryId(99),
            directory.join(focus),
            EntryKind::File,
        )),
    }
}

#[test]
fn single_focus_and_group_selection_are_captured_literally_without_focus_substitution() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["selected & $x;", "focus"] {
        fs::write(temp.path().join(name), b"data").unwrap();
    }
    let request = UserCommandRequest {
        captured: None,
        id: UserCommandRequestId(1),
        command: UserCommandId::new("tool").unwrap(),
        configuration: configuration(
            &std::env::current_exe().unwrap(),
            &["${file.name}", "${selected.paths}"],
            UserCommandLaunch::Single,
        ),
        context: capture(temp.path(), &["selected & $x;"], "focus"),
    };
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(prepared.processes.len(), 1);
    if let UserCommandProcessInvocation::Program { arguments, .. } =
        &prepared.processes[0].invocation
    {
        assert_eq!(
            arguments,
            &[
                "focus".into(),
                temp.path().join("selected & $x;").into_os_string()
            ]
        );
    } else {
        panic!("unexpected terminal plan");
    }
    fs::remove_file(temp.path().join("selected & $x;")).unwrap();
    assert!(
        prepared
            .revalidate(&service, &Cancellation::default())
            .is_err()
    );
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
}

#[test]
fn per_item_prepares_every_plan_and_refuses_a_later_missing_input_before_any_spawn() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["first", "second", "focus"] {
        fs::write(temp.path().join(name), b"data").unwrap();
    }
    let mut request = UserCommandRequest {
        captured: None,
        id: UserCommandRequestId(2),
        command: UserCommandId::new("tool").unwrap(),
        configuration: configuration(
            &std::env::current_exe().unwrap(),
            &["${file.path}"],
            UserCommandLaunch::PerItem,
        ),
        context: capture(temp.path(), &["first", "second"], "focus"),
    };
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(prepared.processes.len(), 2);
    fs::remove_file(temp.path().join("second")).unwrap();
    assert!(
        prepared
            .revalidate_for(0, &service, &Cancellation::default())
            .is_ok()
    );
    assert!(
        prepared
            .revalidate_for(1, &service, &Cancellation::default())
            .is_err()
    );
    drop(prepared);
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
    request.context.active = None;
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
}

#[test]
fn terminal_source_is_literal_and_only_declared_captured_values_become_native_environment() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("focus"), b"data").unwrap();
    let mut commands = (*configuration(
        &std::env::current_exe().unwrap(),
        &[],
        UserCommandLaunch::Single,
    ))
    .clone();
    commands[0].invocation = UserCommandInvocation::Terminal {
        shell: ShellFlavor::PowerShell,
        script: "'${user.authored.script}' ; $env:ME_FILE_PATH".into(),
        variables: vec![UserCommandVariable::FilePath],
    };
    let mut request = UserCommandRequest {
        captured: None,
        id: UserCommandRequestId(3),
        command: UserCommandId::new("tool").unwrap(),
        configuration: Arc::new(commands),
        context: capture(temp.path(), &[], "focus"),
    };
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert!(
        matches!(&prepared.processes[0].invocation, UserCommandProcessInvocation::Terminal { script, environment, .. }
        if script == "'${user.authored.script}' ; $env:ME_FILE_PATH" && environment == &vec![("ME_FILE_PATH".to_owned(), temp.path().join("focus").into_os_string())])
    );
    drop(prepared);
    request.context.focus = None;
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
}

#[test]
fn grouped_selection_can_exceed_stored_template_count_without_truncating_native_argv() {
    let temp = tempfile::tempdir().unwrap();
    let entries = (0..65)
        .map(|index| {
            let path = temp.path().join(format!("item-{index}"));
            fs::write(&path, b"").unwrap();
            Entry::new(EntryId(index), path, EntryKind::File)
        })
        .collect::<Vec<_>>();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let selection =
        Arc::new(SelectionSnapshot::from_bits(view.clone(), vec![u64::MAX, 1]).unwrap());
    let request = UserCommandRequest {
        captured: None,
        id: UserCommandRequestId(4),
        command: UserCommandId::new("tool").unwrap(),
        configuration: configuration(
            &std::env::current_exe().unwrap(),
            &["${selected.paths}"],
            UserCommandLaunch::Single,
        ),
        context: UserCommandContext {
            active: Some(temp.path().to_owned()),
            other: None,
            files: Some(SelectedFiles {
                selection,
                view,
                fallback: None,
            }),
            focus: None,
        },
    };
    let prepared = prepare::prepare(
        &request,
        &midnight_platform::LocalFileSystem::new(None),
        &Cancellation::default(),
    )
    .unwrap();
    if let UserCommandProcessInvocation::Program { arguments, .. } =
        &prepared.processes[0].invocation
    {
        assert_eq!(arguments.len(), 65);
        assert_eq!(arguments[64], entries[64].path.as_os_str());
    } else {
        panic!("unexpected terminal");
    }
}

#[test]
fn disabled_command_preview_is_allowed_but_execution_remains_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let mut commands = (*configuration(
        &std::env::current_exe().unwrap(),
        &["literal"],
        UserCommandLaunch::Single,
    ))
    .clone();
    commands[0].enabled = false;
    let request = UserCommandRequest {
        captured: None,
        id: UserCommandRequestId(5),
        command: UserCommandId::new("tool").unwrap(),
        configuration: Arc::new(commands),
        context: UserCommandContext {
            active: Some(temp.path().to_owned()),
            other: None,
            files: None,
            focus: None,
        },
    };
    let native = midnight_platform::LocalFileSystem::new(None);
    let preview = prepare::prepare_preview(&request, &native, &Cancellation::default())
        .unwrap()
        .into_preview(UserCommandRetention::new(()))
        .unwrap();
    assert_eq!(preview.processes.len(), 1);
    assert!(prepare::prepare(&request, &native, &Cancellation::default()).is_err());
}

/// Fake native pins that count live directory/target handles the way Linux
/// counts open descriptors, and can replace or retype a captured target.
mod counted_pins {
    use midnight_core::*;
    use std::{
        collections::HashSet,
        ffi::OsStr,
        hash::{Hash, Hasher},
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    #[derive(Default)]
    pub struct Pins {
        live: AtomicUsize,
        peak: AtomicUsize,
        replaced: Mutex<HashSet<PathBuf>>,
        directories: Mutex<HashSet<PathBuf>>,
    }
    impl Pins {
        pub fn live(&self) -> usize {
            self.live.load(Ordering::SeqCst)
        }
        pub fn peak(&self) -> usize {
            self.peak.load(Ordering::SeqCst)
        }
        pub fn replace(&self, path: PathBuf) {
            self.replaced.lock().unwrap().insert(path);
        }
        pub fn retype_as_directory(&self, path: PathBuf) {
            self.directories.lock().unwrap().insert(path);
        }
        fn identity(&self, path: &Path) -> TransferIdentity {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            path.hash(&mut hash);
            let mut object = [0; 16];
            object[..8].copy_from_slice(&hash.finish().to_le_bytes());
            object[8] = u8::from(self.replaced.lock().unwrap().contains(path));
            TransferIdentity { device: 1, object }
        }
    }
    struct Live(Arc<Pins>);
    impl Live {
        fn new(pins: &Arc<Pins>) -> Self {
            let live = pins.live.fetch_add(1, Ordering::SeqCst) + 1;
            pins.peak.fetch_max(live, Ordering::SeqCst);
            Self(pins.clone())
        }
    }
    impl Drop for Live {
        fn drop(&mut self) {
            self.0.live.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub struct Service(pub Arc<Pins>);
    impl TransferService for Service {
        fn prepare_transfer_source(
            &self,
            _: &Path,
            _: &Cancellation,
        ) -> Result<Box<dyn TransferSource>, FsError> {
            panic!("a command plan never reads sources")
        }
        fn prepare_transfer_destination(
            &self,
            path: &Path,
            _: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            Ok(Arc::new(Directory {
                path: path.to_owned(),
                identity: self.0.identity(path),
                pins: self.0.clone(),
                _live: Live::new(&self.0),
            }))
        }
    }
    struct Directory {
        path: PathBuf,
        identity: TransferIdentity,
        pins: Arc<Pins>,
        _live: Live,
    }
    impl TransferDirectory for Directory {
        fn path(&self) -> &Path {
            &self.path
        }
        fn identity(&self) -> TransferIdentity {
            self.identity
        }
        fn prepare_target(
            &self,
            name: &OsStr,
            _: &Cancellation,
        ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
            let path = self.path.join(name);
            let kind = if self.pins.directories.lock().unwrap().contains(&path) {
                EntryKind::Directory
            } else if name == "link" {
                EntryKind::Symlink
            } else {
                EntryKind::File
            };
            Ok(Some(Arc::new(Target {
                entry: TransferEntry {
                    identity: self.pins.identity(&path),
                    item: MutationItem {
                        path,
                        kind,
                        metadata: EntryMetadata::default(),
                    },
                },
                pins: self.pins.clone(),
                _live: Live::new(&self.pins),
            })))
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            Ok(())
        }
        fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
            panic!("unexpected inspection")
        }
        fn reject_source_ancestor(
            &self,
            _: TransferIdentity,
            _: &Cancellation,
        ) -> Result<(), FsError> {
            panic!("unexpected transfer")
        }
        fn create_directory(
            &self,
            _: &OsStr,
            _: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            panic!("unexpected mutation")
        }
        fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
            panic!("unexpected stage")
        }
    }
    struct Target {
        entry: TransferEntry,
        pins: Arc<Pins>,
        _live: Live,
    }
    impl TransferTarget for Target {
        fn entry(&self) -> &TransferEntry {
            &self.entry
        }
        fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
            if self.pins.identity(&self.entry.item.path) == self.entry.identity {
                Ok(())
            } else {
                Err(FsError::from_io(
                    "fake pin",
                    &self.entry.item.path,
                    std::io::ErrorKind::InvalidData.into(),
                ))
            }
        }
    }
}

fn many_selected(
    directory: &Path,
    entries: Vec<Entry>,
    launch: UserCommandLaunch,
) -> UserCommandRequest {
    let count = entries.len();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let mut bits = vec![u64::MAX; count.div_ceil(64)];
    if !count.is_multiple_of(64) {
        *bits.last_mut().unwrap() = (1 << (count % 64)) - 1;
    }
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap());
    UserCommandRequest {
        captured: None,
        id: UserCommandRequestId(6),
        command: UserCommandId::new("tool").unwrap(),
        configuration: configuration(&std::env::current_exe().unwrap(), &["${file.path}"], launch),
        context: UserCommandContext {
            active: Some(directory.to_owned()),
            other: None,
            files: Some(SelectedFiles {
                selection,
                view,
                fallback: None,
            }),
            focus: None,
        },
    }
}

fn fake_root() -> &'static Path {
    Path::new(if cfg!(windows) {
        r"C:\fake-pins"
    } else {
        "/fake-pins"
    })
}

#[test]
fn large_selection_holds_a_bounded_number_of_native_pins_and_revalidates_released_ones() {
    let directory = fake_root();
    let entries = (0..600)
        .map(|index| {
            Entry::new(
                EntryId(index),
                directory.join(format!("item-{index}")),
                EntryKind::File,
            )
        })
        .collect::<Vec<_>>();
    let request = many_selected(directory, entries, UserCommandLaunch::PerItem);
    let pins = Arc::new(counted_pins::Pins::default());
    let service = counted_pins::Service(pins.clone());
    let cancel = Cancellation::default();
    let prepared = prepare::prepare(&request, &service, &cancel).unwrap();
    assert_eq!(prepared.processes.len(), 600);
    // The common Linux descriptor soft limit is 1024; one plan holds at most
    // 256 pins, plus one transient directory/target pair while re-pinning.
    assert_eq!(pins.live(), 256);
    assert!(pins.peak() <= 258, "peak {}", pins.peak());
    prepared.revalidate(&service, &cancel).unwrap();
    for index in 0..600 {
        prepared.revalidate_for(index, &service, &cancel).unwrap();
    }
    assert!(pins.peak() <= 258, "peak {}", pins.peak());
    assert_eq!(pins.live(), 256);
    // Held and released targets both reject a replaced object.
    pins.replace(directory.join("item-599"));
    pins.replace(directory.join("item-0"));
    assert!(prepared.revalidate_for(599, &service, &cancel).is_err());
    assert!(prepared.revalidate_for(0, &service, &cancel).is_err());
    assert!(prepared.revalidate_for(300, &service, &cancel).is_ok());
    assert!(prepared.revalidate(&service, &cancel).is_err());
    drop(prepared);
    assert_eq!(pins.live(), 0);
}

#[test]
fn flat_selection_rejects_a_file_replaced_by_a_directory_and_the_reverse() {
    let directory = fake_root();
    let entries = [
        Entry::new(EntryId(1), directory.join("report.txt"), EntryKind::File),
        Entry::new(EntryId(2), directory.join("folder"), EntryKind::Directory),
        Entry::new(EntryId(3), directory.join("link"), EntryKind::DirectoryLink),
    ];
    let cancel = Cancellation::default();
    let request = |selected: usize| {
        many_selected(
            directory,
            vec![entries[selected].clone()],
            UserCommandLaunch::PerItem,
        )
    };
    let pins = Arc::new(counted_pins::Pins::default());
    let service = counted_pins::Service(pins.clone());
    // The fake reports every target as a regular file unless retyped.
    assert!(prepare::prepare(&request(0), &service, &cancel).is_ok());
    pins.retype_as_directory(directory.join("report.txt"));
    let error = prepare::prepare(&request(0), &service, &cancel)
        .err()
        .unwrap();
    assert_eq!(error.kind, std::io::ErrorKind::InvalidData);
    let error = prepare::prepare(&request(1), &service, &cancel)
        .err()
        .unwrap();
    assert_eq!(error.kind, std::io::ErrorKind::InvalidData);
    pins.retype_as_directory(directory.join("folder"));
    assert!(prepare::prepare(&request(1), &service, &cancel).is_ok());
    // A directory-link row pinned as a link stays tolerated, as before:
    // listings classify a link by its target.
    assert!(prepare::prepare(&request(2), &service, &cancel).is_ok());
}
