//! Show hidden files through the real Engine on Linux and macOS: native listings,
//! re-sorts, archive listings and inline trees leave dot-names out and count them.
#![cfg(unix)]
use midnight_core::*;
use midnight_engine::Engine;
use std::{collections::BTreeSet, path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

struct Files;
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        let rows: &[(u64, &str, EntryKind)] = if path.ends_with("Parent") {
            &[
                (10, ".inner", EntryKind::Directory),
                (11, "child", EntryKind::Directory),
                (12, "文档", EntryKind::Directory),
            ]
        } else {
            &[
                (1, ".hidden", EntryKind::File),
                (2, ".config", EntryKind::Directory),
                (3, "Parent", EntryKind::Directory),
                (4, "visible.txt", EntryKind::File),
            ]
        };
        for (id, name, kind) in rows {
            if !emit(Entry::new(EntryId(*id), path.join(name), *kind)) {
                break;
            }
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("metadata was complete")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 100,
            available: 50,
        })
    }
}

#[derive(Debug)]
struct Archive;
impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        78
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Ok([(2, ".git"), (3, "Shown"), (4, ".env")]
            .into_iter()
            .map(|(id, caption)| ArchiveNode {
                id: VirtualEntryId {
                    mount: 78,
                    entry: id,
                },
                parent: directory,
                raw_name: Arc::from(caption.as_bytes()),
                display_name: caption.into(),
                kind: if id == 4 {
                    EntryKind::File
                } else {
                    EntryKind::Directory
                },
                size: None,
                modified: None,
                readable: false,
                access: Default::default(),
            })
            .collect())
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("listing never reads archive content")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

struct Harness {
    commands: midnight_engine::CommandSender,
    events: mpsc::Receiver<Event>,
    engine: tokio::task::JoinHandle<()>,
    directory: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let (send, events) = mpsc::channel(64);
        let mut engine = Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(Files),
            send,
        );
        let (commands, receive) = engine.command_channel(32).unwrap();
        let engine = tokio::spawn(engine.run(receive));
        Self {
            commands,
            events,
            engine,
            directory: tempfile::tempdir().unwrap(),
        }
    }
    async fn event(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(5), self.events.recv())
            .await
            .expect("event timeout")
            .expect("event stream")
    }
    async fn command(&self, command: Command) {
        self.commands.send(command).await.unwrap();
    }
    async fn show_hidden(&self, show: bool) {
        self.command(Command::ShowHiddenEntries(show)).await;
    }
    async fn listing(&mut self) -> TreeRootSnapshot {
        loop {
            if let Event::ListingFinished {
                pane,
                navigation,
                entries,
                index,
            } = self.event().await
            {
                return TreeRootSnapshot {
                    pane,
                    navigation,
                    entries,
                    index,
                };
            }
        }
    }
    async fn navigate(&mut self, navigation: u64) -> TreeRootSnapshot {
        self.command(Command::Navigate {
            pane: PaneId::Left,
            navigation: NavigationId(navigation),
            path: self.directory.path().to_owned(),
            sort: SortSpec::default(),
        })
        .await;
        self.listing().await
    }
    async fn tree(&mut self) -> TreeEvent {
        loop {
            if let Event::Tree(event) = self.event().await {
                return *event;
            }
        }
    }
    async fn tree_command(&self, command: TreeCommand) {
        self.command(Command::Tree(Box::new(command))).await;
    }
    /// A maintenance refresh of `view`; returns the last acknowledged publication.
    async fn refresh(&mut self, view: Arc<TreeProjection>, request: u64) -> Arc<TreeProjection> {
        let request = TreeRefreshRequest {
            view: view.clone(),
            request: TreeRequestId(request),
        };
        self.tree_command(TreeCommand::RefreshLoaded(request)).await;
        let mut latest = view;
        loop {
            match self.tree().await {
                TreeEvent::Published(publication) => {
                    self.tree_command(TreeCommand::Commit(publication.view.clone()))
                        .await;
                    latest = publication.view;
                }
                TreeEvent::Refreshed { result, .. } => {
                    assert!(result.is_ok(), "{result:?}");
                    return latest;
                }
                TreeEvent::RefreshRejected { .. } => panic!("the refresh was rejected"),
                _ => {}
            }
        }
    }
    async fn finish(self) {
        self.command(Command::Shutdown).await;
        tokio::time::timeout(Duration::from_secs(5), self.engine)
            .await
            .unwrap()
            .unwrap();
    }
}

fn names(entries: &[Entry]) -> BTreeSet<String> {
    entries
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listings_leave_dot_names_out_count_them_and_list_them_again_when_shown() {
    let mut h = Harness::new();
    let shown = h.navigate(1).await;
    assert_eq!(
        names(&shown.entries),
        set(&[".config", ".hidden", "Parent", "visible.txt"])
    );
    assert_eq!(shown.index.hidden, 0);

    h.show_hidden(false).await;
    let hidden = h.navigate(2).await;
    assert_eq!(names(&hidden.entries), set(&["Parent", "visible.txt"]));
    assert_eq!(hidden.index.hidden, 2);
    assert_eq!(hidden.index.by_id.len(), 2);

    // A re-sort reuses the listed rows and keeps their hidden count.
    h.command(Command::Resort {
        pane: PaneId::Left,
        navigation: NavigationId(3),
        path: h.directory.path().to_owned(),
        sort: SortSpec {
            column: SortColumn::Name,
            descending: true,
        },
        source: Some(VisibleDirectory {
            pane: PaneId::Left,
            navigation: hidden.navigation,
            path: h.directory.path().to_owned(),
            entries: hidden.entries.clone(),
        }),
    })
    .await;
    let resorted = h.listing().await;
    assert_eq!(names(&resorted.entries), set(&["Parent", "visible.txt"]));
    assert_eq!(resorted.index.hidden, 2);

    h.show_hidden(true).await;
    let again = h.navigate(4).await;
    assert_eq!(names(&again.entries), names(&shown.entries));
    assert_eq!(again.index.hidden, 0);
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archive_listings_leave_dot_named_members_out() {
    let mut h = Harness::new();
    h.show_hidden(false).await;
    let container = h.directory.path().join("fixture.zip");
    h.command(Command::ArchiveNavigate {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        sort: SortSpec::default(),
        replaces: None,
        action: ArchiveAction::List(ArchiveLocation {
            mount: Some(Arc::new(Archive)),
            directory: 1,
            trail: vec![(1, "fixture.zip".into())],
            address: ArchiveAddress {
                provider: "test.archive".into(),
                format: Some(ArchiveFormat::Zip),
                container,
                source_version: vec![1],
                components: vec![],
            },
        }),
    })
    .await;
    let listing = loop {
        if let Event::ArchiveListed { result, .. } = h.event().await {
            break result.unwrap();
        }
    };
    assert_eq!(names(&listing.entries), set(&["Shown"]));
    assert_eq!(listing.index.hidden, 2);
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_trees_leave_hidden_rows_out_and_list_them_after_the_choice_changes() {
    let mut h = Harness::new();
    h.show_hidden(false).await;
    let root = h.navigate(1).await;
    h.tree_command(TreeCommand::Enable {
        root,
        selection_revision: SelectionRevision(0),
        selected: TreeRootSelection::Empty,
    })
    .await;
    let TreeEvent::Published(first) = h.tree().await else {
        panic!("initial tree expected")
    };
    h.tree_command(TreeCommand::Commit(first.view.clone()))
        .await;
    let parent = first
        .view
        .rows
        .iter()
        .find(|row| row.entry.display_name == "Parent")
        .unwrap()
        .id;
    h.tree_command(TreeCommand::Branch {
        view: first.view.clone(),
        request: TreeRequestId(1),
        row: parent,
        action: TreeBranchAction::Expand,
    })
    .await;
    let expanded = loop {
        let TreeEvent::Published(publication) = h.tree().await else {
            panic!("expansion expected")
        };
        h.tree_command(TreeCommand::Commit(publication.view.clone()))
            .await;
        // A branch may publish while it loads; wait for its complete listing.
        if publication.view.rows.iter().any(|row| {
            row.entry.display_name == "Parent" && row.disclosure == TreeDisclosure::Expanded
        }) {
            break publication;
        }
    };
    let nested = |view: &TreeProjection| {
        view.rows
            .iter()
            .filter(|row| row.depth == 1)
            .map(|row| row.entry.display_name.clone())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(nested(&expanded.view), set(&["child", "文档"]));
    assert_eq!(
        expanded.view.hidden, 3,
        "the root listing's two and the expanded folder's one"
    );
    assert!(
        !expanded
            .view
            .rows
            .iter()
            .any(|row| row.entry.display_name.starts_with('.'))
    );

    // The next build after the choice changes lists the loaded hidden rows again.
    h.show_hidden(true).await;
    h.tree_command(TreeCommand::Reproject(TreeReprojectRequest {
        view: expanded.view.clone(),
        request: TreeRequestId(2),
        selection_revision: expanded.selection_revision,
        selection: expanded.selection.clone(),
        options: TreeProjectionOptions::default(),
    }))
    .await;
    let event = h.tree().await;
    let TreeEvent::Reprojected { publication, .. } = event else {
        panic!("reprojection expected, got {event:?}")
    };
    assert_eq!(nested(&publication.view), set(&[".inner", "child", "文档"]));
    assert_eq!(publication.view.hidden, 0);
    h.tree_command(TreeCommand::Commit(publication.view.clone()))
        .await;

    // The panel re-reads a tree with a maintenance refresh. It applies a changed
    // choice even when no listing changed, and lists root-level hidden entries,
    // which the root listing the tree started from had left out.
    let root_names = |view: &TreeProjection| {
        view.rows
            .iter()
            .filter(|row| row.depth == 0)
            .map(|row| row.entry.display_name.clone())
            .collect::<BTreeSet<_>>()
    };
    h.show_hidden(false).await;
    let hidden = h.refresh(publication.view.clone(), 3).await;
    assert_eq!(nested(&hidden), set(&["child", "文档"]));
    assert_eq!(root_names(&hidden), set(&["Parent", "visible.txt"]));
    assert_eq!(hidden.hidden, 3);
    h.show_hidden(true).await;
    let shown = h.refresh(hidden, 4).await;
    assert_eq!(nested(&shown), set(&[".inner", "child", "文档"]));
    assert_eq!(
        root_names(&shown),
        set(&[".config", ".hidden", "Parent", "visible.txt"])
    );
    assert_eq!(shown.hidden, 0);
    h.finish().await;
}
