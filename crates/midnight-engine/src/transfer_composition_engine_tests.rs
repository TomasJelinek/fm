//! The engine composition hands its locale to every prompt that displays transfer
//! entries. Each test drives one prompt through `Engine::run`, composed like the
//! application (audited native transfers and mutations, audited locale), and fails
//! when that prompt's composition line drops the locale: Copy/Move through the
//! audited installation, Rename and Make File List through the engine locale.
//! The native transfer adapters report raw times, so only these lines format them.
use super::CountingLocale;
use crate::{CommandSender, Engine};
use midnight_core::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

/// Elapses only on failure.
const OBSERVE: Duration = Duration::from_secs(30);

struct Composed {
    commands: CommandSender,
    events: mpsc::Receiver<Event>,
    run: JoinHandle<()>,
    locale: Arc<CountingLocale>,
    root: PathBuf,
    _temp: tempfile::TempDir,
}

impl Composed {
    /// As the application installs its native object, with a counting audited
    /// locale in place of the native one.
    fn new() -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
        fs::create_dir_all(&parent).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("transfer-display-")
            .tempdir_in(fs::canonicalize(parent).unwrap())
            .unwrap();
        let native = Arc::new(midnight_platform::LocalFileSystem::new(None));
        let (sender, events) = mpsc::channel(256);
        let mut engine = Engine::new(Engine::lifetime().unwrap(), native.clone(), sender);
        engine
            .configure_audited_transfers()
            .unwrap()
            .install(native.clone());
        engine.configure_mutations().unwrap().install(native);
        let locale = Arc::new(CountingLocale::default());
        engine
            .configure_audited_locale()
            .unwrap()
            .install(locale.clone());
        let (commands, receiver) = engine.command_channel(16).unwrap();
        Self {
            commands,
            events,
            run: tokio::spawn(engine.run(receiver)),
            locale,
            root: temp.path().to_owned(),
            _temp: temp,
        }
    }

    async fn send(&self, command: Command) {
        self.commands.send(command).await.unwrap();
    }

    /// The first event `select` accepts; other events are skipped.
    async fn next<T>(&mut self, mut select: impl FnMut(Event) -> Option<T>) -> T {
        tokio::time::timeout(OBSERVE, async {
            loop {
                let event = self.events.recv().await.expect("engine event channel");
                if let Some(value) = select(event) {
                    return value;
                }
            }
        })
        .await
        .expect("the engine must publish the awaited event")
    }

    /// Retires the engine; the fixture directory stays until the test ends.
    async fn close(&mut self) {
        self.commands.send(Command::Shutdown).await.unwrap();
        // Later events are not awaited; a closed channel refuses them instead.
        self.events.close();
        tokio::time::timeout(OBSERVE, &mut self.run)
            .await
            .expect("the engine must retire")
            .unwrap();
    }
}

/// A displayed entry keeps its raw time and shows the installed locale's text.
fn assert_displayed(item: &MutationItem) {
    assert!(item.metadata.modified.is_some(), "{item:?}");
    assert_eq!(
        item.metadata.localized_modified,
        Some(CountingLocale::text()),
        "{item:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_copy_conflict_prompt_formats_both_entries_through_the_audited_locale() {
    let mut engine = Composed::new();
    let source = engine.root.join("report.txt");
    let destination = engine.root.join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"new").unwrap();
    fs::write(destination.join("report.txt"), b"existing").unwrap();
    engine
        .send(Command::EnqueueCopy(CopyRequest {
            id: OperationId(1),
            start_policy: TransferStartPolicy::Immediate,
            verification: TransferVerification::default(),
            clipboard: FileClipboard {
                paths: vec![source.clone()],
                intent: FileClipboardIntent::Copy,
            },
            destination: destination.clone(),
        }))
        .await;
    let conflict = engine
        .next(|event| match event {
            Event::Copy(CopyEvent::Conflict(conflict)) => Some(conflict),
            Event::Copy(CopyEvent::Progress(progress)) if progress.state.is_terminal() => {
                panic!("expected a conflict prompt: {progress:?}")
            }
            _ => None,
        })
        .await;
    assert_eq!(conflict.source.path, source);
    assert_displayed(&conflict.source);
    assert_displayed(&conflict.target);
    assert_eq!(
        engine.locale.calls(),
        2,
        "only the prompt's two entries are formatted"
    );
    engine
        .send(Command::DecideCopy(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Skip,
        }))
        .await;
    let done = engine
        .next(|event| match event {
            Event::Copy(CopyEvent::Progress(progress)) if progress.state.is_terminal() => {
                Some(progress)
            }
            _ => None,
        })
        .await;
    assert_eq!(done.skipped, 1, "{done:?}");
    assert_eq!(
        fs::read(destination.join("report.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(engine.locale.calls(), 2);
    engine.close().await;
}

/// A captured Rename source, as the tree panel supplies one.
#[derive(Debug)]
struct Captured(PathBuf);
impl CapturedRenameTarget for Captured {
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
    fn row(&self) -> TreeRowId {
        TreeRowId(1)
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        service.prepare(&self.0, cancel)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_rename_conflict_prompt_formats_both_entries_through_the_engine_locale() {
    let mut engine = Composed::new();
    let selected = engine.root.join("selected.txt");
    let existing = engine.root.join("existing.txt");
    fs::write(&selected, b"selected").unwrap();
    fs::write(&existing, b"existing").unwrap();
    engine
        .send(Command::PrepareCapturedRename(PrepareCapturedRename {
            id: OperationId(1),
            source: Arc::new(Captured(selected.clone())),
            source_directory: engine.root.clone(),
        }))
        .await;
    engine
        .next(|event| match event {
            Event::Operation(OperationEvent::Preview(_)) => Some(()),
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state.is_terminal() =>
            {
                panic!("expected a Rename confirmation: {progress:?}")
            }
            _ => None,
        })
        .await;
    assert_eq!(engine.locale.calls(), 0, "preparation formats nothing");
    engine
        .send(Command::ConfirmOperation {
            id: OperationId(1),
            new_name: Some("existing.txt".into()),
        })
        .await;
    let conflict = engine
        .next(|event| match event {
            Event::Operation(OperationEvent::RenameConflict(conflict)) => Some(conflict),
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state.is_terminal() =>
            {
                panic!("expected a conflict prompt: {progress:?}")
            }
            _ => None,
        })
        .await;
    assert_eq!(conflict.source.path, selected);
    assert_displayed(&conflict.source);
    assert_displayed(&conflict.target);
    assert_eq!(engine.locale.calls(), 2);
    engine
        .send(Command::DecideRename(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Skip,
        }))
        .await;
    let done = engine
        .next(|event| match event {
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state.is_terminal() =>
            {
                Some(progress)
            }
            _ => None,
        })
        .await;
    assert_eq!(done.state, OperationState::Cancelled, "{done:?}");
    assert_eq!(fs::read(&selected).unwrap(), b"selected");
    assert_eq!(fs::read(&existing).unwrap(), b"existing");
    assert_eq!(engine.locale.calls(), 2);
    engine.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_file_list_save_confirmation_formats_the_existing_output_through_the_engine_locale()
{
    let mut engine = Composed::new();
    let output = engine.root.join("list.txt");
    fs::write(&output, b"existing").unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        engine.root.join("listed.txt"),
        EntryKind::File,
    )]
    .into();
    let request = FileListRequest {
        id: DesktopRequestId(1),
        generation: 1,
        template: "$(FileName)$(LF)".into(),
        destination: FileListDestination::File {
            path: output.clone(),
            append: false,
        },
        capture: Arc::new(FileListCapture {
            directory: String::new(),
            source: FileListSource::Native {
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                selection: Arc::default(),
                fallback: Some(EntryId(1)),
            },
        }),
    };
    engine.send(Command::RunFileList(Box::new(request))).await;
    let (entry, append) = engine
        .next(|event| match event {
            Event::FileList(event) => match event.result {
                Ok(FileListUpdate::Conflict { entry, append }) => Some((entry, append)),
                result => panic!("expected a save confirmation: {result:?}"),
            },
            _ => None,
        })
        .await;
    assert!(!append);
    assert_eq!(entry.item.path.file_name(), output.file_name());
    assert_displayed(&entry.item);
    assert_eq!(
        engine.locale.calls(),
        1,
        "only the displayed output is formatted"
    );
    engine
        .send(Command::CancelFileList {
            id: DesktopRequestId(1),
            generation: 1,
        })
        .await;
    engine.close().await;
    assert_eq!(fs::read(&output).unwrap(), b"existing");
}
