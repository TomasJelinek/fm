use super::*;
use midnight_core::TransferService;
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
};
use std::{
    fs,
    io::{Read, Seek, Write},
    os::windows::ffi::OsStringExt,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn temporary() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[derive(Default)]
struct AnnouncementEvents {
    roots: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    root_index: Mutex<HashMap<usize, usize>>,
    aliases: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    alias_identity: Mutex<HashSet<usize>>,
    turns: Mutex<Vec<Vec<usize>>>,
    required_before_private: Mutex<Vec<usize>>,
    private_receipts: Mutex<Vec<Vec<usize>>>,
    root_refreshes: AtomicUsize,
    alias_refreshes: AtomicUsize,
    private_reservations: AtomicUsize,
    unqualified_roots: AtomicUsize,
    unknown: AtomicUsize,
    diagnostics: AtomicUsize,
}
impl AnnouncementEvents {
    fn register(&self, pin: Arc<dyn TransferNamespacePin>) -> usize {
        match pin.refresh(&Cancellation::default()).unwrap() {
            TransferNamespaceProbe::Known(scopes) => assert!(!scopes.is_empty()),
            TransferNamespaceProbe::Unknown => {
                self.unqualified_roots.fetch_add(1, Ordering::SeqCst);
            }
        }
        let mut roots = self.roots.lock().unwrap();
        let index = roots.len();
        self.root_index
            .lock()
            .unwrap()
            .insert(Arc::as_ptr(&pin) as *const () as usize, index);
        roots.push(pin);
        index
    }
    fn qualified(&self) -> bool {
        self.unqualified_roots.load(Ordering::SeqCst) == 0
    }
    fn child(&self, directory: &dyn TransferDirectory, name: &str, kind: EntryKind) -> usize {
        self.register(
            directory
                .native_child_namespace_scope(OsStr::new(name), kind, &Cancellation::default())
                .unwrap()
                .unwrap(),
        )
    }
    fn source(&self, path: &Path) -> usize {
        self.register(
            crate::WindowsFileSystem::new(None)
                .prepare_transfer_source(path, &Cancellation::default())
                .unwrap()
                .native_namespace_scope(&Cancellation::default())
                .unwrap()
                .unwrap(),
        )
    }
}
struct AnnouncementContext(Arc<AnnouncementEvents>);
struct AnnouncementTurn {
    events: Arc<AnnouncementEvents>,
    roots: Vec<usize>,
}
impl TransferNamespaceChanges for AnnouncementContext {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        Ok(Box::new(AnnouncementTurn {
            events: self.0.clone(),
            roots: Vec::new(),
        }))
    }
}
impl TransferNamespaceChange for AnnouncementTurn {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        let Some(pin) = pin else {
            self.events.unknown.fetch_add(1, Ordering::SeqCst);
            return Err(invalid(
                Path::new("announcement fixture"),
                "unknown scope refused before mutation",
            ));
        };
        let root = self
            .events
            .root_index
            .lock()
            .unwrap()
            .get(&(Arc::as_ptr(&pin) as *const () as usize))
            .copied();
        if let Some(root) = root {
            pin.refresh(cancel)?;
            self.events.root_refreshes.fetch_add(1, Ordering::SeqCst);
            if !self.roots.contains(&root) {
                self.roots.push(root);
            }
        } else {
            let key = Arc::as_ptr(&pin) as *const () as usize;
            if self.events.alias_identity.lock().unwrap().contains(&key) {
                pin.refresh(cancel)?;
                return Ok(true);
            }
            if self.events.qualified() {
                for root in self.events.required_before_private.lock().unwrap().iter() {
                    assert!(
                        self.roots.contains(root),
                        "compound endpoints must precede private creation"
                    );
                }
            }
            pin.refresh(cancel)?;
            self.events
                .private_receipts
                .lock()
                .unwrap()
                .push(self.roots.clone());
            self.events
                .private_reservations
                .fetch_add(1, Ordering::SeqCst);
            self.events.alias_identity.lock().unwrap().insert(key);
            self.events.aliases.lock().unwrap().push(pin);
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        self.events.turns.lock().unwrap().push(self.roots.clone());
        // Exact original Arc receipts determine this fixture's touched pass. A helper
        // which announces only fresh private children still pays the full root pass.
        let roots = {
            let roots = self.events.roots.lock().unwrap();
            if self.roots.is_empty() {
                roots.clone()
            } else {
                self.roots
                    .iter()
                    .map(|index| roots[*index].clone())
                    .collect()
            }
        };
        for root in roots {
            self.events.root_refreshes.fetch_add(1, Ordering::SeqCst);
            if let Err(error) = root.refresh(&Cancellation::default()) {
                self.events.diagnostics.fetch_add(1, Ordering::SeqCst);
                return Err(error);
            }
        }
        let aliases = std::mem::take(&mut *self.events.aliases.lock().unwrap());
        let mut remaining = aliases.into_iter();
        let mut live = Vec::new();
        while let Some(pin) = remaining.next() {
            self.events.alias_refreshes.fetch_add(1, Ordering::SeqCst);
            match pin.refresh(&Cancellation::default()) {
                Ok(TransferNamespaceProbe::Known(scopes)) if scopes.is_empty() => {
                    self.events
                        .alias_identity
                        .lock()
                        .unwrap()
                        .remove(&(Arc::as_ptr(&pin) as *const () as usize));
                    drop(pin);
                }
                Ok(_) => live.push(pin),
                Err(error) => {
                    live.push(pin);
                    live.extend(remaining);
                    *self.events.aliases.lock().unwrap() = live;
                    self.events.diagnostics.fetch_add(1, Ordering::SeqCst);
                    return Err(error);
                }
            }
        }
        *self.events.aliases.lock().unwrap() = live;
        Ok(())
    }
}
fn announcement_directory(path: &Path) -> destination::Directory {
    destination::Directory::from_file(native::open_parent(path).unwrap(), path.to_owned()).unwrap()
}

#[test]
fn native_file_stage_lifecycles_announce_exact_original_arcs_with_linear_root_refreshes() {
    let temp = temporary();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        const COUNT: usize = 64;
        for index in 0..COUNT {
            events.child(&directory, &format!("selected-{index}"), EntryKind::File);
        }
        for _ in 0..COUNT {
            let mut stage = directory.stage_file(&cancel).unwrap();
            stage.write_all(b"native stage bytes").unwrap();
            stage.cleanup().unwrap();
            drop(stage);
            assert!(events.aliases.lock().unwrap().len() <= 1);
        }
        // A final actual directory lifecycle retires the last file's metadata pin;
        // file cleanup itself returned while its Stage still held the DELETE handle.
        let recovery = change(&cancel, |mutation| {
            destination::recovery::source_stage_in(&directory, &cancel, mutation)
        })
        .unwrap();
        recovery.remove_empty_stage().unwrap();
        assert!(events.aliases.lock().unwrap().is_empty());
        assert_eq!(
            events.private_reservations.load(Ordering::SeqCst),
            COUNT + 1
        );
        let turns = events.turns.lock().unwrap();
        assert_eq!(turns.len(), 2 * COUNT + 2);
        if events.qualified() {
            assert!(turns.iter().all(|roots| !roots.is_empty()));
            assert!(events.root_refreshes.load(Ordering::SeqCst) <= 4 * COUNT + 4);
        } else {
            // No test-made lookup mode: an unqualified real volume keeps legacy
            // full refresh. Build must qualify the positive branch on native NTFS.
            assert!(turns.iter().all(Vec::is_empty));
            assert!(events.root_refreshes.load(Ordering::SeqCst) >= 2 * COUNT * COUNT);
        }
        let job = current_job().unwrap();
        let index = job.index.lock().unwrap();
        assert_eq!(index.pins.len(), COUNT);
        assert_eq!(index.roots.len(), COUNT);
        assert_eq!(index.parents.len(), 1);
        assert!(index.objects.is_empty());
        assert_eq!(job.owners.lock().unwrap().len(), 1);
    });
}

#[test]
fn materialized_file_and_subtree_targets_keep_their_original_announcement_arcs() {
    let temp = temporary();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        events.child(&directory, "sentinel", EntryKind::File);
        let file = events.child(&directory, "file", EntryKind::File);
        let tree = events.child(&directory, "tree", EntryKind::Directory);
        let mut stage = directory.stage_file(&cancel).unwrap();
        stage.write_all(b"published bytes").unwrap();
        stage
            .publish_no_replace(OsStr::new("file"), &cancel)
            .unwrap();
        stage.cleanup().unwrap();
        drop(stage);
        let child = directory
            .create_directory(OsStr::new("tree"), &cancel)
            .unwrap();
        let mut nested = child.stage_file(&cancel).unwrap();
        nested.write_all(b"nested bytes").unwrap();
        nested
            .publish_no_replace(OsStr::new("child"), &cancel)
            .unwrap();
        nested.cleanup().unwrap();
        drop(nested);
        if events.qualified() {
            let turns = events.turns.lock().unwrap();
            assert!(turns[1].contains(&file));
            assert!(turns[2].contains(&tree));
            assert!(turns[3].contains(&tree));
            assert!(turns[4].contains(&tree));
        }
        assert_eq!(
            fs::read(temp.path().join("file")).unwrap(),
            b"published bytes"
        );
        assert_eq!(
            fs::read(temp.path().join("tree/child")).unwrap(),
            b"nested bytes"
        );
    });
}

#[test]
fn source_capture_announces_original_object_aliases_before_creating_private_recovery() {
    let temp = temporary();
    let source_directory = temp.path().join("protected");
    {
        let parent = native::open_parent(temp.path()).unwrap();
        let private =
            destination::recovery::create_private_directory(&parent, OsStr::new("protected"), 0)
                .unwrap();
        drop(private);
    }
    fs::write(source_directory.join("source"), b"source bytes").unwrap();
    fs::hard_link(
        source_directory.join("source"),
        source_directory.join("alias"),
    )
    .unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let service = crate::WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let prepared = service
            .prepare_move_source(&source_directory.join("source"), &cancel)
            .unwrap();
        let source = events.register(
            prepared
                .source
                .native_namespace_scope(&cancel)
                .unwrap()
                .unwrap(),
        );
        let alias = events.source(&source_directory.join("alias"));
        *events.required_before_private.lock().unwrap() = vec![source, alias];
        assert!(matches!(
            prepared.capture.capture(&cancel).unwrap(),
            midnight_core::TransferMoveCaptureOutcome::Captured { .. }
        ));
        if events.qualified() {
            let receipts = events.private_receipts.lock().unwrap();
            assert!(!receipts.is_empty());
            assert!(receipts[0].contains(&source) && receipts[0].contains(&alias));
        }
        assert_eq!(
            fs::read(source_directory.join("alias")).unwrap(),
            b"source bytes"
        );
    });
}

#[test]
fn native_replacement_declares_both_endpoints_and_displaced_aliases_before_private_creation() {
    let temp = temporary();
    fs::write(temp.path().join("source"), b"replacement").unwrap();
    fs::write(temp.path().join("target"), b"original").unwrap();
    fs::hard_link(temp.path().join("target"), temp.path().join("alias")).unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let service = crate::WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let prepared = service
            .prepare_move_source(&temp.path().join("source"), &cancel)
            .unwrap();
        let source = events.register(
            prepared
                .source
                .native_namespace_scope(&cancel)
                .unwrap()
                .unwrap(),
        );
        let directory = announcement_directory(temp.path());
        let target = events.child(&directory, "target", EntryKind::File);
        let alias = events.source(&temp.path().join("alias"));
        let expected = directory
            .prepare_target(OsStr::new("target"), &cancel)
            .unwrap()
            .unwrap();
        *events.required_before_private.lock().unwrap() = vec![source, target, alias];
        let outcome = prepared
            .capture
            .try_native_move(
                &directory,
                OsStr::new("target"),
                Some((
                    expected,
                    midnight_core::TransferReplacementPolicy::RecoverableCurrentEntry,
                )),
                &cancel,
            )
            .unwrap();
        assert!(matches!(
            outcome,
            midnight_core::TransferNativeMoveOutcome::Moved { .. }
        ));
        if events.qualified() {
            let receipts = events.private_receipts.lock().unwrap();
            assert!(!receipts.is_empty());
            assert!(
                [source, target, alias]
                    .iter()
                    .all(|root| receipts[0].contains(root))
            );
        }
        assert_eq!(
            fs::read(temp.path().join("target")).unwrap(),
            b"replacement"
        );
        assert_eq!(fs::read(temp.path().join("alias")).unwrap(), b"original");
    });
}

#[test]
fn replacement_publication_declares_displaced_roots_before_its_private_directory() {
    let temp = temporary();
    fs::write(temp.path().join("target"), b"original").unwrap();
    fs::hard_link(temp.path().join("target"), temp.path().join("alias")).unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        events.child(&directory, "sentinel", EntryKind::File);
        let target = events.child(&directory, "target", EntryKind::File);
        let alias = events.source(&temp.path().join("alias"));
        let expected = directory
            .prepare_target(OsStr::new("target"), &cancel)
            .unwrap()
            .unwrap();
        let mut stage = directory
            .stage_file_for_target(
                OsStr::new("target"),
                expected,
                midnight_core::TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel,
            )
            .unwrap();
        stage.write_all(b"replacement").unwrap();
        let private_before = events.private_receipts.lock().unwrap().len();
        *events.required_before_private.lock().unwrap() = vec![target, alias];
        assert!(matches!(
            stage
                .publish_replace(OsStr::new("target"), &cancel)
                .unwrap(),
            midnight_core::TransferPublication::Replaced { .. }
        ));
        if events.qualified() {
            let receipts = events.private_receipts.lock().unwrap();
            assert!(receipts.len() > private_before);
            assert!(receipts[private_before].contains(&target));
            assert!(receipts[private_before].contains(&alias));
        }
        assert_eq!(
            fs::read(temp.path().join("target")).unwrap(),
            b"replacement"
        );
        assert_eq!(fs::read(temp.path().join("alias")).unwrap(), b"original");
    });
}

#[test]
fn directory_native_move_announces_selected_descendant_parent_groups_and_preserves_commit() {
    let temp = temporary();
    fs::create_dir(temp.path().join("source")).unwrap();
    fs::write(temp.path().join("source/child"), b"child bytes").unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let service = crate::WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let prepared = service
            .prepare_move_source(&temp.path().join("source"), &cancel)
            .unwrap();
        let source = events.register(
            prepared
                .source
                .native_namespace_scope(&cancel)
                .unwrap()
                .unwrap(),
        );
        let descendant = events.source(&temp.path().join("source/child"));
        let directory = announcement_directory(temp.path());
        let target = events.child(&directory, "moved", EntryKind::Directory);
        let outcome = prepared
            .capture
            .try_native_move(&directory, OsStr::new("moved"), None, &cancel)
            .unwrap();
        assert!(matches!(
            outcome,
            midnight_core::TransferNativeMoveOutcome::Moved { .. }
        ));
        if events.qualified() {
            let turns = events.turns.lock().unwrap();
            assert!(
                [source, descendant, target]
                    .iter()
                    .all(|root| turns[0].contains(root))
            );
        }
        // Strict original-parent witnesses can now fail refresh. That diagnostic
        // cannot replace the actual successful native directory rename with Err.
        assert!(events.diagnostics.load(Ordering::SeqCst) > 0);
        assert_eq!(
            fs::read(temp.path().join("moved/child")).unwrap(),
            b"child bytes"
        );
    });
}

#[test]
fn private_pins_cannot_replace_dropped_original_roots_as_the_next_sentinel() {
    let temp = temporary();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        events.child(&directory, "old", EntryKind::File);
        let mut stage = directory.stage_file(&cancel).unwrap();
        let retired_roots = std::mem::take(&mut *events.roots.lock().unwrap());
        events.root_index.lock().unwrap().clear();
        drop(retired_roots);
        events.turns.lock().unwrap().clear();
        stage.cleanup().unwrap();
        drop(stage);
        let recovery = change(&cancel, |mutation| {
            destination::recovery::source_stage_in(&directory, &cancel, mutation)
        })
        .unwrap();
        recovery.remove_empty_stage().unwrap();
        assert!(events.turns.lock().unwrap().iter().all(Vec::is_empty));
        assert_eq!(events.unknown.load(Ordering::SeqCst), 0);
        let index = current_job().unwrap();
        let index = index.index.lock().unwrap();
        assert!(index.roots.is_empty());
        assert!(index.parents.is_empty());
        assert!(index.ancestors.is_empty());
    });
}

#[test]
fn incomplete_later_endpoint_cannot_continue_an_opted_turn_or_erase_its_native_stage() {
    let temp = temporary();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        events.child(&directory, "selected", EntryKind::File);
        let (stage, failure) = change(&cancel, |mutation| {
            let stage = destination::recovery::source_stage_in(&directory, &cancel, mutation)?;
            let failure = mutation
                .create(
                    directory.native_file(),
                    directory.path(),
                    OsStr::new("unregistered"),
                    Creation::Planned,
                    &cancel,
                    || {
                        destination::recovery::create_private_file_for_namespace_test(
                            directory.native_file(),
                            OsStr::new("unregistered"),
                        )
                    },
                )
                .err();
            Ok((stage, failure))
        })
        .unwrap();
        assert!(stage.path().is_dir());
        if events.qualified() {
            assert!(failure.is_some());
            assert_eq!(events.unknown.load(Ordering::SeqCst), 1);
            assert!(!temp.path().join("unregistered").exists());
        }
        stage.remove_empty_stage().unwrap();
    });
}

#[test]
fn incomplete_first_endpoint_keeps_later_private_creation_on_full_refresh() {
    let temp = temporary();
    fs::write(temp.path().join("unregistered"), b"unchanged source").unwrap();
    let foreign = File::open(temp.path().join("unregistered")).unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        events.child(&directory, "selected", EntryKind::File);
        let stage = change(&cancel, |mutation| {
            // A real held endpoint lacks this job's binding coverage. Preparing it
            // must conservatively prevent a later private helper from opting in.
            mutation.prepare_file(&foreign, &cancel)?;
            destination::recovery::source_stage_in(&directory, &cancel, mutation)
        })
        .unwrap();
        assert!(stage.path().is_dir());
        assert!(events.turns.lock().unwrap()[0].is_empty());
        assert_eq!(events.unknown.load(Ordering::SeqCst), 0);
        stage.remove_empty_stage().unwrap();
        assert_eq!(
            fs::read(temp.path().join("unregistered")).unwrap(),
            b"unchanged source"
        );
    });
}

#[derive(Default)]
struct TouchedAliasEvents {
    originals: Mutex<BTreeMap<usize, Arc<dyn TransferNamespacePin>>>,
    aliases: Mutex<BTreeMap<usize, Arc<dyn TransferNamespacePin>>>,
    alias_identity: Mutex<HashMap<usize, usize>>,
    alias_facts: Mutex<HashMap<usize, TransferNamespaceProbe>>,
    receipts: Mutex<Vec<(Vec<usize>, Vec<usize>)>>,
    next_alias: AtomicUsize,
    root_refreshes: AtomicUsize,
    alias_refreshes: AtomicUsize,
    reannouncements: AtomicUsize,
    certified_reannouncements: AtomicUsize,
    unqualified: AtomicBool,
    diagnostics: AtomicUsize,
}
impl TouchedAliasEvents {
    fn register(&self, pin: Arc<dyn TransferNamespacePin>) {
        self.root_refreshes.fetch_add(1, Ordering::SeqCst);
        match pin.refresh(&Cancellation::default()).unwrap() {
            TransferNamespaceProbe::Known(scopes) => assert!(!scopes.is_empty()),
            TransferNamespaceProbe::Unknown => self.unqualified.store(true, Ordering::SeqCst),
        }
        self.originals
            .lock()
            .unwrap()
            .insert(Arc::as_ptr(&pin) as *const () as usize, pin);
    }
    fn child(&self, directory: &dyn TransferDirectory, name: &str, kind: EntryKind) {
        self.register(
            directory
                .native_child_namespace_scope(OsStr::new(name), kind, &Cancellation::default())
                .unwrap()
                .unwrap(),
        );
    }
    fn qualified(&self) -> bool {
        !self.unqualified.load(Ordering::SeqCst)
    }
    fn probes(&self) -> usize {
        self.root_refreshes.load(Ordering::SeqCst) + self.alias_refreshes.load(Ordering::SeqCst)
    }
    fn retains_object(&self, object: TransferIdentity) -> bool {
        self.alias_facts.lock().unwrap().values().any(|probe| {
            matches!(probe, TransferNamespaceProbe::Known(scopes)
                if scopes.iter().any(|scope| scope.object == Some(object)))
        })
    }
    fn refresh(&self, selected: Option<(&HashSet<usize>, &HashSet<usize>)>) -> Result<(), FsError> {
        let roots: Vec<_> = {
            let roots = self.originals.lock().unwrap();
            match selected {
                Some((selected, _)) => selected.iter().map(|id| roots[id].clone()).collect(),
                None => roots.values().cloned().collect(),
            }
        };
        let aliases: Vec<_> = {
            let aliases = self.aliases.lock().unwrap();
            match selected {
                Some((_, selected)) => selected
                    .iter()
                    .map(|id| (*id, aliases[id].clone()))
                    .collect(),
                None => aliases.iter().map(|(id, pin)| (*id, pin.clone())).collect(),
            }
        };
        let cancel = Cancellation::default();
        for pin in roots {
            self.root_refreshes.fetch_add(1, Ordering::SeqCst);
            if let Err(error) = pin.refresh(&cancel) {
                self.diagnostics.fetch_add(1, Ordering::SeqCst);
                return Err(error);
            }
        }
        for (id, pin) in aliases {
            self.alias_refreshes.fetch_add(1, Ordering::SeqCst);
            let probe = match pin.refresh(&cancel) {
                Ok(probe) => probe,
                Err(error) => {
                    self.diagnostics.fetch_add(1, Ordering::SeqCst);
                    return Err(error);
                }
            };
            if matches!(&probe, TransferNamespaceProbe::Known(scopes) if scopes.is_empty()) {
                let retired = self.aliases.lock().unwrap().remove(&id);
                self.alias_identity
                    .lock()
                    .unwrap()
                    .remove(&(Arc::as_ptr(&pin) as *const () as usize));
                self.alias_facts.lock().unwrap().remove(&id);
                drop(retired);
            } else {
                self.alias_facts.lock().unwrap().insert(id, probe);
            }
        }
        Ok(())
    }
    fn release_pins(&self) {
        let roots = std::mem::take(&mut *self.originals.lock().unwrap());
        let aliases = std::mem::take(&mut *self.aliases.lock().unwrap());
        self.alias_identity.lock().unwrap().clear();
        self.alias_facts.lock().unwrap().clear();
        drop(roots);
        drop(aliases);
    }
}
struct TouchedAliasContext(Arc<TouchedAliasEvents>);
struct TouchedAliasTurn {
    events: Arc<TouchedAliasEvents>,
    roots: HashSet<usize>,
    aliases: HashSet<usize>,
}
impl TransferNamespaceChanges for TouchedAliasContext {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        Ok(Box::new(TouchedAliasTurn {
            events: self.0.clone(),
            roots: HashSet::new(),
            aliases: HashSet::new(),
        }))
    }
}
impl TransferNamespaceChange for TouchedAliasTurn {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        let Some(pin) = pin else {
            return Err(invalid(
                Path::new("touched alias fixture"),
                "unknown announcement",
            ));
        };
        let key = Arc::as_ptr(&pin) as *const () as usize;
        if self.events.originals.lock().unwrap().contains_key(&key) {
            self.roots.insert(key);
            self.events.root_refreshes.fetch_add(1, Ordering::SeqCst);
            pin.refresh(cancel)?;
        } else {
            let existing = self
                .events
                .alias_identity
                .lock()
                .unwrap()
                .get(&key)
                .copied();
            let id = if let Some(id) = existing {
                assert!(Arc::ptr_eq(&self.events.aliases.lock().unwrap()[&id], &pin));
                self.events.reannouncements.fetch_add(1, Ordering::SeqCst);
                id
            } else {
                let id = self.events.next_alias.fetch_add(1, Ordering::SeqCst);
                self.events.aliases.lock().unwrap().insert(id, pin.clone());
                self.events.alias_identity.lock().unwrap().insert(key, id);
                id
            };
            self.aliases.insert(id);
            self.events.alias_refreshes.fetch_add(1, Ordering::SeqCst);
            let probe = pin.refresh(cancel)?;
            if matches!(&probe, TransferNamespaceProbe::Known(scopes) if scopes.is_empty()) {
                assert!(existing.is_some(), "a fresh private pin cannot start empty");
                self.events
                    .certified_reannouncements
                    .fetch_add(1, Ordering::SeqCst);
            }
            self.events.alias_facts.lock().unwrap().insert(id, probe);
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        self.events.receipts.lock().unwrap().push((
            self.roots.iter().copied().collect(),
            self.aliases.iter().copied().collect(),
        ));
        // Only an actual exact original receipt permits the partial host pass.
        // A fresh/private receipt alone leaves the existing full-refresh fallback.
        let selected = (!self.roots.is_empty()).then_some((&self.roots, &self.aliases));
        self.events.refresh(selected)
    }
}
struct NamespaceTestFile {
    file: File,
    _owner: NativeOwner,
}
impl NamespaceTestFile {
    fn open(directory: &destination::Directory, name: &str) -> Self {
        let file = native::open_relative(
            directory.native_file(),
            OsStr::new(name),
            windows_sys::Win32::Storage::FileSystem::DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        )
        .unwrap();
        let owner = owner_for(&file, &directory.path().join(name));
        Self {
            file,
            _owner: owner,
        }
    }
}

#[test]
fn published_private_aliases_reannounce_exact_arcs_without_probing_unrelated_history() {
    let temp = temporary();
    fs::create_dir(temp.path().join("outputs")).unwrap();
    fs::create_dir(temp.path().join("outside")).unwrap();
    let events = Arc::new(TouchedAliasEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(TouchedAliasContext(events.clone()));
    let retired_job = with_transfer_namespace_changes(context, || {
        let parent = announcement_directory(temp.path());
        let directory = announcement_directory(&temp.path().join("outputs"));
        let cancel = Cancellation::default();
        events.child(&parent, "outputs", EntryKind::Directory);
        let job = current_job().unwrap();
        const COUNT: usize = 64;
        let mut held = None;
        let mut first_object = None;
        for index in 0..COUNT {
            let mut stage = directory.stage_file(&cancel).unwrap();
            assert_eq!(events.next_alias.load(Ordering::SeqCst), index + 1);
            stage.write_all(b"retained published bytes").unwrap();
            let guard = stage.rewind_for_validation(&cancel).unwrap();
            stage
                .publish_no_replace(OsStr::new(&format!("output-{index}")), &cancel)
                .unwrap();
            let object = stage.revalidate_publication(&cancel).unwrap();
            drop(stage);
            assert!(
                !job.owners_closed(object),
                "the real readback guard still owns the file"
            );
            assert_eq!(events.aliases.lock().unwrap().len(), index + 1);
            if events.qualified() {
                assert!(events.retains_object(object));
                assert!(
                    events
                        .receipts
                        .lock()
                        .unwrap()
                        .last()
                        .unwrap()
                        .1
                        .contains(&index)
                );
            }
            if index == 0 {
                first_object = Some(object);
                held = Some(guard);
            } else {
                drop(guard);
                assert!(job.owners_closed(object));
            }
        }
        let first_object = first_object.unwrap();
        assert!(!job.owners_closed(first_object));
        let bytes = fs::read(directory.path().join("output-0")).unwrap();
        assert_eq!(bytes, b"retained published bytes");
        drop(held);
        assert!(job.owners_closed(first_object));
        // Publication aliases remain live claims after normal actual last-close.
        assert_eq!(events.aliases.lock().unwrap().len(), COUNT);
        if events.qualified() {
            assert!(events.reannouncements.load(Ordering::SeqCst) >= COUNT);
            assert!(events.probes() <= 12 * COUNT + 8);
        }

        // Real outside hard-link identity survives a full admission-style refresh.
        // The engine tests own the grant decision; this adapter must retain its fact.
        fs::hard_link(
            directory.path().join("output-0"),
            temp.path().join("outside/alias"),
        )
        .unwrap();
        events.refresh(None).unwrap();
        let peer_events = Arc::new(TouchedAliasEvents::default());
        let peer: Arc<dyn TransferNamespaceChanges> = Arc::new(TouchedAliasContext(peer_events));
        let peer_probe = with_transfer_namespace_changes(peer, || {
            let source = crate::WindowsFileSystem::new(None)
                .prepare_transfer_source(&temp.path().join("outside/alias"), &cancel)
                .unwrap();
            source
                .native_namespace_scope(&cancel)
                .unwrap()
                .unwrap()
                .refresh(&cancel)
                .unwrap()
        });
        if let TransferNamespaceProbe::Known(scopes) = peer_probe {
            assert_eq!(scopes[0].object, Some(first_object));
            assert!(events.retains_object(first_object));
        }
        let outside = announcement_directory(&temp.path().join("outside"));
        let prospective = outside
            .native_child_namespace_scope(OsStr::new("future"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        if let TransferNamespaceProbe::Known(scopes) = prospective.refresh(&cancel).unwrap() {
            assert!(scopes[0].object.is_none());
            assert!(!outside.path().join("future").exists());
            assert!(events.retains_object(first_object));
        }
        drop(prospective);
        drop(outside);

        let alias_count = events.next_alias.load(Ordering::SeqCst);
        let before = events.probes();
        let old = NamespaceTestFile::open(&directory, "output-0");
        change(&cancel, |mutation| {
            mutation
                .rename(
                    &old.file,
                    directory.native_file(),
                    OsStr::new("renamed"),
                    &cancel,
                )
                .map_err(|error| super::error(directory.path(), error))
        })
        .unwrap();
        if events.qualified() {
            assert!(
                events
                    .receipts
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .1
                    .contains(&0)
            );
            assert!(events.probes() - before <= 4);
        }
        assert_eq!(events.next_alias.load(Ordering::SeqCst), alias_count);
        assert_eq!(fs::read(directory.path().join("renamed")).unwrap(), bytes);
        change(&cancel, |mutation| {
            mutation
                .remove(&old.file, &cancel, || native::remove(&old.file))
                .map_err(|error| super::error(directory.path(), error))
        })
        .unwrap();
        if events.qualified() {
            assert!(
                events
                    .receipts
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .1
                    .contains(&0)
            );
        }
        drop(old);
        events.refresh(None).unwrap();
        assert_eq!(events.aliases.lock().unwrap().len(), COUNT - 1);
        assert_eq!(events.alias_identity.lock().unwrap().len(), COUNT - 1);
        assert_eq!(events.diagnostics.load(Ordering::SeqCst), 0);
        let retired = Arc::downgrade(&job);
        drop(job);
        events.release_pins();
        drop(directory);
        drop(parent);
        retired
    });
    assert!(retired_job.upgrade().is_none());
}

#[test]
fn closed_private_alias_reannouncement_precedes_same_slot_prospective_create() {
    let temp = temporary();
    let events = Arc::new(TouchedAliasEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(TouchedAliasContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        events.child(&directory, "sentinel", EntryKind::File);
        let job = current_job().unwrap();
        let mut stage = directory.stage_file(&cancel).unwrap();
        let key = Arc::as_ptr(&events.aliases.lock().unwrap()[&0]) as *const () as usize;
        let weak = job.index.lock().unwrap().pins[&key].weak.clone();
        let private = weak.upgrade().unwrap();
        let initial = private.bindings.lock().unwrap()[0].clone();
        let guard = stage.rewind_for_validation(&cancel).unwrap();
        stage.cleanup().unwrap();
        drop(stage);
        assert!(!job.owners_closed(initial.object.unwrap()));
        drop(guard);
        assert!(job.owners_closed(initial.object.unwrap()));
        if !events.qualified() {
            // The native unknown mode keeps full refresh; it cannot activate this
            // exact-Arc incremental edge. Retire the completed alias before reuse.
            events.refresh(None).unwrap();
        }
        let replacement = change(&cancel, |mutation| {
            let file = mutation
                .create_relative(
                    directory.native_file(),
                    &initial.name,
                    EntryKind::File,
                    &cancel,
                    || {
                        native::open_relative(
                            directory.native_file(),
                            &initial.name,
                            FILE_READ_DATA
                                | FILE_WRITE_DATA
                                | FILE_READ_ATTRIBUTES
                                | DELETE
                                | SYNCHRONIZE,
                            FILE_SHARE_READ,
                            windows_sys::Wdk::Storage::FileSystem::FILE_CREATE,
                            windows_sys::Wdk::Storage::FileSystem::FILE_NON_DIRECTORY_FILE,
                        )
                    },
                )
                .map_err(|failure| error(directory.path(), failure))?;
            let owner = owner_for(&file, &directory.path().join(&initial.name));
            Ok(NamespaceTestFile {
                file,
                _owner: owner,
            })
        })
        .unwrap();
        assert_eq!(events.next_alias.load(Ordering::SeqCst), 2);
        assert_eq!(
            events
                .aliases
                .lock()
                .unwrap()
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [1]
        );
        if events.qualified() {
            assert_eq!(events.certified_reannouncements.load(Ordering::SeqCst), 1);
            assert!(
                events
                    .receipts
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .1
                    .contains(&0)
            );
            assert!(
                events
                    .receipts
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .1
                    .contains(&1)
            );
        }
        change(&cancel, |mutation| {
            mutation
                .remove(&replacement.file, &cancel, || {
                    native::remove(&replacement.file)
                })
                .map_err(|failure| error(directory.path(), failure))
        })
        .unwrap();
        drop(replacement);
        events.refresh(None).unwrap();
        assert!(events.aliases.lock().unwrap().is_empty());
        assert_eq!(events.diagnostics.load(Ordering::SeqCst), 0);
        drop(initial);
        drop(private);
        drop(job);
        events.release_pins();
    });
}

#[test]
fn directory_move_announces_exact_retained_private_descendants_before_native_commit() {
    let temp = temporary();
    fs::create_dir(temp.path().join("source")).unwrap();
    let events = Arc::new(TouchedAliasEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(TouchedAliasContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let child = announcement_directory(&temp.path().join("source"));
        let cancel = Cancellation::default();
        events.child(&directory, "sentinel", EntryKind::File);
        let mut stage = child.stage_file(&cancel).unwrap();
        stage.write_all(b"private descendant bytes").unwrap();
        stage
            .publish_no_replace(OsStr::new("leaf"), &cancel)
            .unwrap();
        drop(stage);
        let service = crate::WindowsFileSystem::new(None);
        let prepared = service.prepare_move_source(child.path(), &cancel).unwrap();
        events.register(
            prepared
                .source
                .native_namespace_scope(&cancel)
                .unwrap()
                .unwrap(),
        );
        events.child(&directory, "moved", EntryKind::Directory);
        let before = events.reannouncements.load(Ordering::SeqCst);
        let outcome = prepared
            .capture
            .try_native_move(&directory, OsStr::new("moved"), None, &cancel)
            .unwrap();
        assert!(matches!(
            outcome,
            midnight_core::TransferNativeMoveOutcome::Moved { .. }
        ));
        if events.qualified() {
            assert_eq!(events.next_alias.load(Ordering::SeqCst), 1);
            assert_eq!(events.reannouncements.load(Ordering::SeqCst), before + 1);
            assert!(
                events
                    .receipts
                    .lock()
                    .unwrap()
                    .last()
                    .unwrap()
                    .1
                    .contains(&0)
            );
        }
        // The old pinned parent path is deliberately strict after the owned move.
        // Its finish diagnostic must preserve both the committed outcome and alias.
        assert!(events.diagnostics.load(Ordering::SeqCst) > 0);
        assert_eq!(events.aliases.lock().unwrap().len(), 1);
        assert_eq!(
            fs::read(temp.path().join("moved/leaf")).unwrap(),
            b"private descendant bytes"
        );
        drop(prepared);
        drop(child);
        events.release_pins();
    });
}

#[test]
#[ignore = "Pending published temporary-alias retirement ruling; separate from increment28 foundation"]
fn published_stages_do_not_accumulate_private_claims_after_actual_owner_close() {
    let temp = temporary();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let directory = announcement_directory(temp.path());
        let cancel = Cancellation::default();
        const COUNT: usize = 64;
        for index in 0..COUNT {
            events.child(&directory, &format!("output-{index}"), EntryKind::File);
        }
        for index in 0..COUNT {
            let mut stage = directory.stage_file(&cancel).unwrap();
            stage.write_all(b"published native bytes").unwrap();
            stage
                .publish_no_replace(OsStr::new(&format!("output-{index}")), &cancel)
                .unwrap();
            drop(stage);
            if events.qualified() {
                assert!(events.aliases.lock().unwrap().len() <= 1);
            }
        }
        let recovery = change(&cancel, |mutation| {
            destination::recovery::source_stage_in(&directory, &cancel, mutation)
        })
        .unwrap();
        recovery.remove_empty_stage().unwrap();
        if events.qualified() {
            assert!(events.aliases.lock().unwrap().is_empty());
            assert!(
                events.root_refreshes.load(Ordering::SeqCst)
                    + events.alias_refreshes.load(Ordering::SeqCst)
                    <= 12 * COUNT + 16
            );
        }
        for index in 0..COUNT {
            assert_eq!(
                fs::read(temp.path().join(format!("output-{index}"))).unwrap(),
                b"published native bytes"
            );
        }
    });
}

#[derive(Default)]
struct Observations {
    begun: AtomicUsize,
    finished: AtomicUsize,
    declined: AtomicBool,
    reservations: AtomicUsize,
    fail_refresh: AtomicBool,
    diagnostics: AtomicUsize,
    pins: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
}
struct Adapter(Arc<Observations>);
struct Change(Arc<Observations>);
impl TransferNamespaceChanges for Adapter {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        self.0.begun.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Change(self.0.clone())))
    }
}
impl TransferNamespaceChange for Change {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        _: &Cancellation,
    ) -> Result<bool, FsError> {
        let pin = pin.expect("native private name needs a pin");
        // Original receipts reuse an already retained capability. They are not a
        // fresh private-name attempt and must not consume the nonce rejection fixture.
        if self
            .0
            .pins
            .lock()
            .unwrap()
            .iter()
            .any(|old| Arc::ptr_eq(old, &pin))
        {
            return Ok(true);
        }
        self.0.reservations.fetch_add(1, Ordering::SeqCst);
        if self.0.declined.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        self.0.pins.lock().unwrap().push(pin);
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        let pins = self.0.pins.lock().unwrap().clone();
        for pin in pins {
            pin.refresh(&Cancellation::default())?;
        }
        self.0.finished.fetch_add(1, Ordering::SeqCst);
        if self.0.fail_refresh.swap(false, Ordering::SeqCst) {
            self.0.diagnostics.fetch_add(1, Ordering::SeqCst);
            return Err(error(
                Path::new(""),
                io::Error::other("retained host refresh diagnostic"),
            ));
        }
        Ok(())
    }
}

#[test]
fn sibling_metadata_pins_share_native_parent_authority_and_keep_no_leaf_handle() {
    let temp = temporary();
    let service = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut pins = Vec::new();
    let first = parent::capture(
        &native::open_parent(temp.path()).unwrap(),
        temp.path(),
        &cancel,
    )
    .unwrap();
    for index in 0..128 {
        let path = temp.path().join(format!("f{index}"));
        fs::write(&path, b"source").unwrap();
        let source = service.prepare_transfer_source(&path, &cancel).unwrap();
        let pin = source.native_namespace_scope(&cancel).unwrap().unwrap();
        let same_parent = parent::capture(
            &native::open_parent(temp.path()).unwrap(),
            temp.path(),
            &cancel,
        )
        .unwrap();
        assert!(Arc::ptr_eq(&first, &same_parent));
        drop(source);
        // A retained Copy source handle denies DELETE sharing. Successful removal proves
        // that the metadata pin did not retain that per-leaf handle.
        fs::remove_file(&path).unwrap();
        assert!(pin.refresh(&cancel).is_err());
        pins.push(pin);
    }
    assert_eq!(pins.len(), 128);
}

#[test]
fn qualified_native_modes_preserve_case_rules_and_refuse_unqualified_prospective_aliases() {
    let insensitive = parent::LookupMode::ShortNamesDisabled {
        case_sensitive: false,
    };
    let sensitive = parent::LookupMode::ShortNamesDisabled {
        case_sensitive: true,
    };
    assert_eq!(
        insensitive.key(OsStr::new("F1")),
        insensitive.key(OsStr::new("f1"))
    );
    assert_ne!(
        insensitive.key(OsStr::new("f1")),
        insensitive.key(OsStr::new("f2"))
    );
    assert_ne!(
        sensitive.key(OsStr::new("F1")),
        sensitive.key(OsStr::new("f1"))
    );
    assert!(insensitive.key(OsStr::new("žluťoučký")).is_none());
    assert!(sensitive.key(OsStr::new("žluťoučký")).is_some());
    let raw = OsString::from_wide(&[0xd800, b'A' as u16]);
    assert_eq!(sensitive.key(&raw).unwrap().as_ref(), [0, 0xd8, b'A', 0]);
    assert!(insensitive.key(&raw).is_none());
    assert!(
        parent::LookupMode::Unqualified
            .key(OsStr::new("f1"))
            .is_none()
    );
    assert!(
        parent::LookupMode::Unqualified
            .key(OsStr::new(".me-0123456789abcdef"))
            .is_none()
    );
}

#[test]
fn actual_hard_link_aliases_keep_the_same_native_object_identity() {
    let temp = temporary();
    let one = temp.path().join("one");
    let alias = temp.path().join("alias");
    fs::write(&one, b"same inode").unwrap();
    fs::hard_link(&one, &alias).unwrap();
    let service = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let first = service.prepare_transfer_source(&one, &cancel).unwrap();
    let second = service.prepare_transfer_source(&alias, &cancel).unwrap();
    let a = first.native_namespace_scope(&cancel).unwrap().unwrap();
    let b = second.native_namespace_scope(&cancel).unwrap().unwrap();
    match (a.refresh(&cancel).unwrap(), b.refresh(&cancel).unwrap()) {
        (TransferNamespaceProbe::Known(a), TransferNamespaceProbe::Known(b)) => {
            assert_eq!(a[0].object, b[0].object);
            assert_ne!(a[0].lookup_key, b[0].lookup_key);
        }
        (TransferNamespaceProbe::Unknown, TransferNamespaceProbe::Unknown) => {}
        _ => panic!("one native parent cannot acquire conflicting lookup qualifications"),
    }
    assert_eq!(first.entry().identity, second.entry().identity);
}

#[test]
fn replaced_parent_binding_is_refused_without_adopting_the_new_directory() {
    let temp = temporary();
    let original = temp.path().join("parent");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("f1"), b"original").unwrap();
    let service = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service
        .prepare_transfer_source(&original.join("f1"), &cancel)
        .unwrap();
    let pin = source.native_namespace_scope(&cancel).unwrap().unwrap();
    drop(source);
    fs::rename(&original, temp.path().join("held-parent")).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("f1"), b"unrelated winner").unwrap();
    assert!(pin.refresh(&cancel).is_err());
    assert_eq!(fs::read(original.join("f1")).unwrap(), b"unrelated winner");
}

#[test]
fn actual_stage_reserves_before_create_and_data_io_never_begins_namespace_turns() {
    let temp = temporary();
    let observations = Arc::new(Observations::default());
    observations.declined.store(true, Ordering::SeqCst);
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let service = crate::WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let directory = service
            .prepare_transfer_destination(temp.path(), &cancel)
            .unwrap();
        let target = directory
            .native_child_namespace_scope(OsStr::new("f1"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        observations.pins.lock().unwrap().push(target);
        let mut stage = directory.stage_file(&cancel).unwrap();
        assert_eq!(observations.reservations.load(Ordering::SeqCst), 2);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        let turns = observations.begun.load(Ordering::SeqCst);
        stage.write_all(b"new verified bytes").unwrap();
        stage.sync(&cancel).unwrap();
        stage.rewind().unwrap();
        let mut read = [0; 18];
        assert_eq!(stage.read(&mut read).unwrap(), 18);
        assert_eq!(observations.begun.load(Ordering::SeqCst), turns);
        stage.publish_no_replace(OsStr::new("f1"), &cancel).unwrap();
        assert_eq!(observations.begun.load(Ordering::SeqCst), turns + 1);
        assert_eq!(observations.finished.load(Ordering::SeqCst), turns + 1);
        assert_eq!(
            fs::read(temp.path().join("f1")).unwrap(),
            b"new verified bytes"
        );
    });
}

#[test]
fn native_move_rebound_destination_updates_only_its_own_job_witnesses() {
    let temp = temporary();
    let source_dir = temp.path().join("S");
    let destination_dir = temp.path().join("D");
    fs::create_dir(&source_dir).unwrap();
    fs::create_dir(&destination_dir).unwrap();
    fs::write(source_dir.join("f1"), b"original bytes").unwrap();
    let service = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let other: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(Arc::default()));
    let unrelated = with_transfer_namespace_changes(other, || {
        service
            .prepare_transfer_source(&source_dir.join("f1"), &cancel)
            .unwrap()
            .native_namespace_scope(&cancel)
            .unwrap()
            .unwrap()
    });
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let prepared = service
            .prepare_move_source(&source_dir.join("f1"), &cancel)
            .unwrap();
        let source_pin = prepared
            .source
            .native_namespace_scope(&cancel)
            .unwrap()
            .unwrap();
        let destination = service
            .prepare_move_destination(&destination_dir, &cancel)
            .unwrap();
        let target_pin = destination
            .native_child_namespace_scope(OsStr::new("f1"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        observations
            .pins
            .lock()
            .unwrap()
            .extend([source_pin, target_pin]);
        let outcome = prepared
            .capture
            .try_native_move(destination.as_ref(), OsStr::new("f1"), None, &cancel)
            .unwrap();
        assert!(matches!(
            outcome,
            midnight_core::TransferNativeMoveOutcome::Moved { .. }
        ));
        assert_eq!(observations.begun.load(Ordering::SeqCst), 1);
        assert_eq!(observations.finished.load(Ordering::SeqCst), 1);
        assert_eq!(
            fs::read(destination_dir.join("f1")).unwrap(),
            b"original bytes"
        );
    });
    assert!(unrelated.refresh(&cancel).is_err());
}

#[test]
fn native_sibling_pairs_qualify_distinct_slots_only_on_the_supported_native_volume_mode() {
    let temp = temporary();
    let source_dir = temp.path().join("S");
    let destination_dir = temp.path().join("D");
    fs::create_dir(&source_dir).unwrap();
    fs::create_dir(&destination_dir).unwrap();
    fs::write(source_dir.join("f1"), b"one").unwrap();
    fs::write(source_dir.join("f2"), b"two").unwrap();
    let cancel = Cancellation::default();
    let service = crate::WindowsFileSystem::new(None);
    let destination = service
        .prepare_transfer_destination(&destination_dir, &cancel)
        .unwrap();
    let mut source = Vec::new();
    let mut target = Vec::new();
    for name in ["f1", "f2"] {
        let prepared = service
            .prepare_transfer_source(&source_dir.join(name), &cancel)
            .unwrap();
        source.push(
            prepared
                .native_namespace_scope(&cancel)
                .unwrap()
                .unwrap()
                .refresh(&cancel)
                .unwrap(),
        );
        target.push(
            destination
                .native_child_namespace_scope(OsStr::new(name), EntryKind::File, &cancel)
                .unwrap()
                .unwrap()
                .refresh(&cancel)
                .unwrap(),
        );
    }
    for pair in [&source, &target] {
        match (&pair[0], &pair[1]) {
            (TransferNamespaceProbe::Known(first), TransferNamespaceProbe::Known(second)) => {
                assert_eq!(first.len(), 1);
                assert_eq!(second.len(), 1);
                assert_eq!(first[0].parent.identity, second[0].parent.identity);
                assert_eq!(
                    first[0].parent.lookup_domain,
                    second[0].parent.lookup_domain
                );
                assert_ne!(first[0].lookup_key, second[0].lookup_key);
            }
            (TransferNamespaceProbe::Unknown, TransferNamespaceProbe::Unknown) => {}
            _ => panic!("siblings must share one native qualification mode"),
        }
    }
    if let (TransferNamespaceProbe::Known(source), TransferNamespaceProbe::Known(target)) =
        (&source[0], &target[0])
    {
        assert_ne!(source[0].parent.identity, target[0].parent.identity);
        assert!(source[0].object.is_some());
        assert!(target[0].object.is_none());
    }
}

#[test]
fn cleanup_keeps_pending_delete_conservative_then_certifies_only_the_owned_alias_absent() {
    let temp = temporary();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let service = crate::WindowsFileSystem::new(None);
        let destination = service
            .prepare_transfer_destination(temp.path(), &cancel)
            .unwrap();
        let mut stage = destination.stage_file(&cancel).unwrap();
        let private = observations.pins.lock().unwrap()[0].clone();
        stage.write_all(b"unpublished data").unwrap();
        let guard = stage.rewind_for_validation(&cancel).unwrap();
        stage.cleanup().unwrap();
        // Native absence alone is insufficient while the stage or an independently
        // returned readback guard still owns its original native handle.
        match private.refresh(&cancel).unwrap() {
            TransferNamespaceProbe::Unknown => {}
            TransferNamespaceProbe::Known(_) => panic!("live stage handle cannot retire"),
        }
        drop(stage);
        match private.refresh(&cancel).unwrap() {
            TransferNamespaceProbe::Unknown => {}
            TransferNamespaceProbe::Known(_) => panic!("live readback guard cannot retire"),
        }
        drop(guard);
        match private.refresh(&cancel).unwrap() {
            TransferNamespaceProbe::Known(scopes) => assert!(scopes.is_empty()),
            TransferNamespaceProbe::Unknown => panic!("closed owned alias must retire"),
        }
        assert_eq!(observations.begun.load(Ordering::SeqCst), 2);
        assert_eq!(observations.finished.load(Ordering::SeqCst), 2);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    });
}

#[test]
fn metadata_pin_keeps_only_weak_job_adapter_identity() {
    let temp = temporary();
    let path = temp.path().join("f1");
    fs::write(&path, b"source").unwrap();
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(Arc::default()));
    let weak = Arc::downgrade(&adapter);
    let pin = with_transfer_namespace_changes(adapter, || {
        crate::WindowsFileSystem::new(None)
            .prepare_transfer_source(&path, &Cancellation::default())
            .unwrap()
            .native_namespace_scope(&Cancellation::default())
            .unwrap()
            .unwrap()
    });
    assert!(weak.upgrade().is_none());
    pin.refresh(&Cancellation::default()).unwrap();
}

#[test]
fn committed_native_publication_survives_a_separately_retained_host_refresh_failure() {
    let temp = temporary();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let destination = crate::WindowsFileSystem::new(None)
            .prepare_transfer_destination(temp.path(), &cancel)
            .unwrap();
        let target = destination
            .native_child_namespace_scope(OsStr::new("final"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        observations.pins.lock().unwrap().push(target);
        let mut stage = destination.stage_file(&cancel).unwrap();
        stage.write_all(b"committed").unwrap();
        observations.fail_refresh.store(true, Ordering::SeqCst);
        let publication = stage
            .publish_no_replace(OsStr::new("final"), &cancel)
            .unwrap();
        assert!(matches!(
            publication,
            midnight_core::TransferPublication::Published(_)
        ));
        assert_eq!(observations.diagnostics.load(Ordering::SeqCst), 1);
        assert_eq!(observations.finished.load(Ordering::SeqCst), 2);
        assert_eq!(fs::read(temp.path().join("final")).unwrap(), b"committed");
    });
}

#[test]
fn owned_case_only_native_move_uses_actual_binding_probes_even_in_an_unknown_alias_mode() {
    let temp = temporary();
    fs::write(temp.path().join("mixed"), b"selected source").unwrap();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let service = crate::WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let source = service
            .prepare_move_source(&temp.path().join("mixed"), &cancel)
            .unwrap();
        let pin = source
            .source
            .native_namespace_scope(&cancel)
            .unwrap()
            .unwrap();
        observations.pins.lock().unwrap().push(pin);
        let directory = service
            .prepare_move_destination(temp.path(), &cancel)
            .unwrap();
        source
            .capture
            .try_native_move(directory.as_ref(), OsStr::new("MIXED"), None, &cancel)
            .unwrap();
        assert_eq!(observations.finished.load(Ordering::SeqCst), 1);
        assert_eq!(
            fs::read(temp.path().join("MIXED")).unwrap(),
            b"selected source"
        );
    });
}

#[test]
fn private_directory_alias_waits_for_the_last_actual_directory_clone() {
    let temp = temporary();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let directory = destination::Directory::from_file(
            native::open_parent(temp.path()).unwrap(),
            temp.path().to_owned(),
        )
        .unwrap();
        let private = change(&cancel, |mutation| {
            destination::recovery::native_move_recovery_in(&directory, &cancel, mutation)
        })
        .unwrap();
        let duplicate = private.clone();
        let pin = observations.pins.lock().unwrap()[0].clone();
        private.remove_empty_stage().unwrap();
        assert!(matches!(
            pin.refresh(&cancel).unwrap(),
            TransferNamespaceProbe::Unknown
        ));
        drop(duplicate);
        match pin.refresh(&cancel).unwrap() {
            TransferNamespaceProbe::Known(scopes) => assert!(scopes.is_empty()),
            TransferNamespaceProbe::Unknown => panic!("last native directory clone has closed"),
        }
    });
}

#[test]
fn child_binding_keeps_removed_parent_close_proof_until_its_own_retirement() {
    let temp = temporary();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let directory = destination::Directory::from_file(
            native::open_parent(temp.path()).unwrap(),
            temp.path().to_owned(),
        )
        .unwrap();
        let private = change(&cancel, |mutation| {
            destination::recovery::native_move_recovery_in(&directory, &cancel, mutation)
        })
        .unwrap();
        let private_id = private.identity();
        let duplicate = private.clone();
        let mut stage = private.stage_file(&cancel).unwrap();
        let child = observations.pins.lock().unwrap()[1].clone();
        stage.cleanup().unwrap();
        drop(stage);
        private.remove_empty_stage().unwrap();
        assert!(matches!(
            child.refresh(&cancel).unwrap(),
            TransferNamespaceProbe::Unknown
        ));
        // Abandon the parent's metadata observer while its actual native clone is
        // still held. The remaining child binding must retain the parent close proof.
        let retired_parent = observations.pins.lock().unwrap().remove(0);
        drop(retired_parent);
        drop(duplicate);
        let job = current_job().unwrap();
        {
            let owners = job.owners.lock().unwrap();
            let parent = owners
                .get(&private_id)
                .expect("child still needs parent close proof");
            assert!(parent.references > 0);
            assert!(parent.states.is_empty());
        }
        match child.refresh(&cancel).unwrap() {
            TransferNamespaceProbe::Known(scopes) => assert!(scopes.is_empty()),
            TransferNamespaceProbe::Unknown => {
                panic!("removed parent and child owners have closed")
            }
        }
        assert!(!job.owners.lock().unwrap().contains_key(&private_id));
        assert!(!job.removed_parent(private_id));
    });
}

#[test]
fn unwinding_native_owner_drop_never_certifies_a_private_alias_retired() {
    let temp = temporary();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_transfer_namespace_changes(adapter, || {
            let cancel = Cancellation::default();
            let destination = crate::WindowsFileSystem::new(None)
                .prepare_transfer_destination(temp.path(), &cancel)
                .unwrap();
            let mut stage = destination.stage_file(&cancel).unwrap();
            stage.cleanup().unwrap();
            panic!("native owner teardown was not confirmed");
        });
    }));
    assert!(panic.is_err());
    let pin = observations.pins.lock().unwrap()[0].clone();
    assert!(matches!(
        pin.refresh(&Cancellation::default()).unwrap(),
        TransferNamespaceProbe::Unknown
    ));
}

#[test]
fn a_later_normal_directory_close_cannot_erase_an_earlier_clone_unwind() {
    let temp = temporary();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let directory = destination::Directory::from_file(
            native::open_parent(temp.path()).unwrap(),
            temp.path().to_owned(),
        )
        .unwrap();
        let private = change(&cancel, |mutation| {
            destination::recovery::native_move_recovery_in(&directory, &cancel, mutation)
        })
        .unwrap();
        let private_id = private.identity();
        let job = current_job().unwrap();
        let duplicate = private.clone();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _unconfirmed = duplicate;
            panic!("one directory clone did not complete ordinary teardown");
        }));
        assert!(panic.is_err());
        private.remove_empty_stage().unwrap();
        let pin = observations.pins.lock().unwrap()[0].clone();
        assert!(matches!(
            pin.refresh(&cancel).unwrap(),
            TransferNamespaceProbe::Unknown
        ));
        assert!(!job.owners_closed(private_id));
        let retired_observer = observations.pins.lock().unwrap().remove(0);
        drop(retired_observer);
        drop(pin);
        let owners = job.owners.lock().unwrap();
        let unresolved = owners
            .get(&private_id)
            .expect("unconfirmed owner must remain");
        assert_eq!(unresolved.references, 0);
        assert!(unresolved.states.values().any(|state| {
            state.unconfirmed.load(Ordering::Acquire) && !state.closed.load(Ordering::Acquire)
        }));
    });
}

#[test]
fn one_live_job_retires_each_native_stage_alias_and_its_index_before_the_next_attempt() {
    fn index_size(job: &Job) -> (usize, usize, usize) {
        let index = job.index.lock().unwrap();
        (
            index.slots.len(),
            index.slots.values().map(HashMap::len).sum(),
            index.pins.len(),
        )
    }
    fn owner_size(job: &Job) -> (usize, usize) {
        let owners = job.owners.lock().unwrap();
        (
            owners.len(),
            owners.values().map(|owners| owners.states.len()).sum(),
        )
    }

    let temp = temporary();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let directory = crate::WindowsFileSystem::new(None)
            .prepare_transfer_destination(temp.path(), &cancel)
            .unwrap();
        let root = directory
            .native_child_namespace_scope(OsStr::new("selected-target"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        observations.pins.lock().unwrap().push(root);
        let job = current_job().unwrap();
        let baseline = index_size(&job);
        assert_eq!(baseline, (1, 1, 1));
        let owner_baseline = owner_size(&job);
        assert_eq!(owner_baseline, (1, 1));

        for _ in 0..64 {
            let mut stage = directory.stage_file(&cancel).unwrap();
            let private = observations.pins.lock().unwrap()[1].clone();
            let guard = stage.rewind_for_validation(&cancel).unwrap();
            assert_eq!(observations.pins.lock().unwrap().len(), 2);
            assert_eq!(
                index_size(&job),
                (baseline.0 + 1, baseline.1 + 1, baseline.2 + 1)
            );
            assert_eq!(
                owner_size(&job),
                (owner_baseline.0 + 1, owner_baseline.1 + 2)
            );
            stage.cleanup().unwrap();
            drop(stage);
            assert!(matches!(
                private.refresh(&cancel).unwrap(),
                TransferNamespaceProbe::Unknown
            ));
            assert_eq!(
                index_size(&job),
                (baseline.0 + 1, baseline.1 + 1, baseline.2 + 1)
            );
            assert_eq!(
                owner_size(&job),
                (owner_baseline.0 + 1, owner_baseline.1 + 1)
            );
            drop(guard);
            match private.refresh(&cancel).unwrap() {
                TransferNamespaceProbe::Known(scopes) => assert!(scopes.is_empty()),
                TransferNamespaceProbe::Unknown => panic!("actual native owners have closed"),
            }
            // The host removes only the certified retired alias, outside its mutex.
            let retired = observations.pins.lock().unwrap().remove(1);
            assert!(Arc::ptr_eq(&retired, &private));
            drop(retired);
            drop(private);
            assert_eq!(observations.pins.lock().unwrap().len(), 1);
            assert_eq!(index_size(&job), baseline);
            assert_eq!(owner_size(&job), owner_baseline);
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);

            // Abandoning a prepared root before any native creation also removes its
            // weak observer bucket; this exercises Pin::Drop independently of refresh.
            let abandoned = directory
                .native_child_namespace_scope(OsStr::new("abandoned"), EntryKind::File, &cancel)
                .unwrap()
                .unwrap();
            assert_eq!(
                index_size(&job),
                (baseline.0 + 1, baseline.1 + 1, baseline.2 + 1)
            );
            drop(abandoned);
            assert_eq!(index_size(&job), baseline);
            assert_eq!(owner_size(&job), owner_baseline);
        }
        assert_eq!(observations.reservations.load(Ordering::SeqCst), 64);
        assert_eq!(observations.begun.load(Ordering::SeqCst), 128);
        assert_eq!(observations.finished.load(Ordering::SeqCst), 128);
    });
}

#[test]
fn modified_time_pin_retains_source_owner_outside_namespace_context() {
    let temp = temporary();
    let path = temp.path().join("source");
    let renamed = temp.path().join("renamed");
    fs::write(&path, b"retained timestamp source").unwrap();
    let cancel = Cancellation::default();
    let observations = Arc::new(Observations::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Adapter(observations));
    let (source, root_pin, job) = with_transfer_namespace_changes(adapter, || {
        let source = crate::WindowsFileSystem::new(None)
            .prepare_transfer_source(&path, &cancel)
            .unwrap();
        let root_pin = source.native_namespace_scope(&cancel).unwrap().unwrap();
        (source, root_pin, current_job().unwrap())
    });
    assert!(
        midnight_platform_common::transfer_namespace::current_transfer_namespace_changes()
            .is_none()
    );
    let id = source.entry().identity;
    assert!(job.index.lock().unwrap().objects.contains_key(&id));
    let timestamp = source.prepare_modified_time(&cancel).unwrap();
    let modified = timestamp.modified();
    assert!(!job.owners_closed(id));
    drop(source);

    timestamp.revalidate(&cancel).unwrap();
    assert_eq!(timestamp.current_modified(&cancel).unwrap(), modified);
    let rename_error = fs::rename(&path, &renamed).unwrap_err();
    assert_eq!(
        rename_error.raw_os_error(),
        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32),
        "{rename_error:?}"
    );
    assert!(
        !job.owners_closed(id),
        "the timestamp pin still retains the original source handle"
    );

    drop(timestamp);
    assert!(job.owners_closed(id));
    fs::rename(&path, &renamed).unwrap();
    assert_eq!(fs::read(&renamed).unwrap(), b"retained timestamp source");
    drop(root_pin);
}
