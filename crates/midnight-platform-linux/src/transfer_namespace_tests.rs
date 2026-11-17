use super::*;
use midnight_core::{TransferNamespaceProbe, TransferPublication, TransferService};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
};
use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct AnnouncementEvents {
    roots: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    root_index: Mutex<HashMap<usize, usize>>,
    aliases: Mutex<BTreeMap<usize, Arc<dyn TransferNamespacePin>>>,
    turns: Mutex<Vec<Vec<usize>>>,
    alias_turns: Mutex<Vec<Vec<usize>>>,
    root_refreshes: AtomicUsize,
    alias_refreshes: AtomicUsize,
    reject_item: std::sync::atomic::AtomicBool,
    unknown: AtomicUsize,
}
impl AnnouncementEvents {
    fn register(&self, pin: Arc<dyn TransferNamespacePin>) {
        assert!(
            !known(pin.as_ref()).is_empty(),
            "actual native qualified root required"
        );
        let mut roots = self.roots.lock().unwrap();
        self.root_index
            .lock()
            .unwrap()
            .insert(Arc::as_ptr(&pin) as *const () as usize, roots.len());
        roots.push(pin);
    }
}
struct AnnouncementContext(Arc<AnnouncementEvents>);
struct AnnouncementTurn {
    events: Arc<AnnouncementEvents>,
    roots: Vec<usize>,
    aliases: Vec<usize>,
}
impl TransferNamespaceChanges for AnnouncementContext {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        Ok(Box::new(AnnouncementTurn {
            events: self.0.clone(),
            roots: Vec::new(),
            aliases: Vec::new(),
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
            let probe = pin.refresh(cancel)?;
            self.events.alias_refreshes.fetch_add(1, Ordering::SeqCst);
            if self.events.reject_item.load(Ordering::SeqCst)
                && matches!(&probe, TransferNamespaceProbe::Known(scopes) if scopes.iter().any(|scope|
                    scope.extent == TransferNamespaceExtent::Entry && scope.lookup_key.as_ref() == b"item"))
            {
                return Err(invalid(
                    Path::new("item"),
                    "fixture refuses recovery item before link",
                ));
            }
            let identity = Arc::as_ptr(&pin) as *const () as usize;
            self.events
                .aliases
                .lock()
                .unwrap()
                .entry(identity)
                .or_insert(pin);
            if !self.aliases.contains(&identity) {
                self.aliases.push(identity);
            }
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        // Mirror strict opt-in: fresh aliases alone require the full root pass.
        let touched: Vec<_> = {
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
        for pin in touched {
            pin.refresh(&Cancellation::default())?;
            self.events.root_refreshes.fetch_add(1, Ordering::SeqCst);
        }
        let aliases: Vec<_> = {
            let aliases = self.events.aliases.lock().unwrap();
            if self.roots.is_empty() {
                aliases.iter().map(|(id, pin)| (*id, pin.clone())).collect()
            } else {
                self.aliases
                    .iter()
                    .map(|id| (*id, aliases[id].clone()))
                    .collect()
            }
        };
        for (id, pin) in aliases {
            let probe = pin.refresh(&Cancellation::default())?;
            self.events.alias_refreshes.fetch_add(1, Ordering::SeqCst);
            if matches!(probe, TransferNamespaceProbe::Known(scopes) if scopes.is_empty()) {
                let retired = self.events.aliases.lock().unwrap().remove(&id);
                drop(retired);
            }
        }
        self.events.turns.lock().unwrap().push(self.roots.clone());
        self.events
            .alias_turns
            .lock()
            .unwrap()
            .push(self.aliases.clone());
        Ok(())
    }
}

#[test]
fn refused_recovery_item_observer_reports_owned_directory_without_publishing_file() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("target"), b"old bytes").unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    events.reject_item.store(true, Ordering::SeqCst);
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        events.register(
            parent
                .native_child_namespace_scope(OsStr::new("sentinel"), EntryKind::File, &cancel)
                .unwrap()
                .unwrap(),
        );
        let target = parent
            .prepare_target(OsStr::new("target"), &cancel)
            .unwrap()
            .unwrap();
        let mut stage = parent
            .stage_file_for_target(
                OsStr::new("target"),
                target,
                midnight_core::TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel,
            )
            .unwrap();
        stage.write_all(b"unpublished replacement").unwrap();
        let result = stage
            .publish_replace(OsStr::new("target"), &cancel)
            .unwrap();
        let TransferPublication::RecoveryRequired {
            published,
            recovery,
            error,
        } = result
        else {
            panic!("the actual owned recovery directory must be reported");
        };
        assert!(published.is_none());
        assert_eq!(recovery.len(), 1);
        assert!(recovery[0].path.is_dir());
        assert!(!recovery[0].path.join("item").exists());
        assert!(recovery[0].identity.is_some());
        assert!(error.message.contains("fixture refuses recovery item"));
        assert_eq!(
            std::fs::read(temp.path().join("target")).unwrap(),
            b"old bytes"
        );
        stage.cleanup().unwrap();
        let pins: Vec<_> = events.aliases.lock().unwrap().values().cloned().collect();
        assert!(
            pins.iter()
                .flat_map(|pin| known(pin.as_ref()).to_vec())
                .any(|scope| scope.object == recovery[0].identity)
        );
    });
}

#[test]
fn actual_published_file_aliases_keep_objects_and_readback_owner_without_history_probes() {
    use std::io::{Read, Write};
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("outside")).unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        events.register(
            parent
                .native_child_namespace_scope(OsStr::new("tree"), EntryKind::Directory, &cancel)
                .unwrap()
                .unwrap(),
        );
        let tree = parent
            .create_directory(OsStr::new("tree"), &cancel)
            .unwrap();
        const COUNT: usize = 32;
        let before_roots = events.root_refreshes.load(Ordering::SeqCst);
        let before_aliases = events.alias_refreshes.load(Ordering::SeqCst);
        let mut held_readback = None;
        let mut first_alias = None;
        for index in 0..COUNT {
            let mut stage = tree.stage_file(&cancel).unwrap();
            stage.write_all(&[index as u8; 64]).unwrap();
            assert!(matches!(
                stage
                    .publish_no_replace(OsStr::new(&format!("file-{index}")), &cancel,)
                    .unwrap(),
                TransferPublication::Published(_)
            ));
            let alias = events.alias_turns.lock().unwrap().last().unwrap().clone();
            assert_eq!(alias.len(), 1);
            if index == 0 {
                first_alias = Some(alias[0]);
                held_readback = Some(stage.rewind_for_validation(&cancel).unwrap());
                let mut bytes = Vec::new();
                stage.read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, vec![0; 64]);
            }
            let turns = events.turns.lock().unwrap().len();
            stage.cleanup().unwrap();
            drop(stage);
            assert_eq!(
                events.turns.lock().unwrap().len(),
                turns,
                "data close stays Stable"
            );
            assert_eq!(events.aliases.lock().unwrap().len(), index + 1);
        }
        assert!(events.root_refreshes.load(Ordering::SeqCst) - before_roots <= 2 * COUNT);
        assert!(events.alias_refreshes.load(Ordering::SeqCst) - before_aliases <= 3 * COUNT);
        held_readback.as_ref().unwrap().revalidate(&cancel).unwrap();
        let original = events.aliases.lock().unwrap()[&first_alias.unwrap()].clone();
        let published = known(original.as_ref());
        assert!(published.iter().any(|scope| scope.object.is_some()));
        std::fs::hard_link(
            temp.path().join("tree/file-0"),
            temp.path().join("outside/alias"),
        )
        .unwrap();
        let outside = directory(&temp.path().join("outside"));
        let peer = outside
            .native_child_namespace_scope(OsStr::new("alias"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        let peer_scope = known(peer.as_ref());
        assert_ne!(published[0].parent.identity, peer_scope[0].parent.identity);
        assert_eq!(published[0].object, peer_scope[0].object);
        drop(held_readback.take());
        assert!(
            !known(original.as_ref()).is_empty(),
            "publication is never an empty certificate"
        );
        drop(original);
        drop(peer);
        drop(tree);
        drop(outside);
        drop(parent);
    });
    let aliases = std::mem::take(&mut *events.aliases.lock().unwrap());
    let weak: Vec<_> = aliases.values().map(Arc::downgrade).collect();
    drop(aliases);
    assert!(weak.iter().all(|pin| pin.upgrade().is_none()));
}

#[test]
fn native_replacement_file_observers_keep_old_and_new_objects_with_real_read_guards_held() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("tree")).unwrap();
    std::fs::create_dir(temp.path().join("outside")).unwrap();
    let target_path = temp.path().join("tree/target");
    std::fs::write(&target_path, b"old bytes").unwrap();
    std::fs::hard_link(&target_path, temp.path().join("outside/old-alias")).unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        events.register(
            parent
                .native_child_namespace_scope(OsStr::new("tree"), EntryKind::Directory, &cancel)
                .unwrap()
                .unwrap(),
        );
        let tree = directory(&temp.path().join("tree"));
        let mut old_source = crate::LinuxFileSystem::new(None)
            .prepare_transfer_source(&target_path, &cancel)
            .unwrap();
        let old_identity = old_source.entry().identity;
        let old_readback = old_source.rewind_for_validation(&cancel).unwrap();
        drop(old_source);
        let target = tree
            .prepare_target(OsStr::new("target"), &cancel)
            .unwrap()
            .unwrap();
        let mut stage = tree
            .stage_file_for_target(
                OsStr::new("target"),
                target,
                midnight_core::TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel,
            )
            .unwrap();
        stage.write_all(b"replacement bytes").unwrap();
        assert!(matches!(
            stage
                .publish_replace(OsStr::new("target"), &cancel)
                .unwrap(),
            TransferPublication::Replaced { .. }
        ));
        let new_readback = stage.rewind_for_validation(&cancel).unwrap();
        let new_identity = stage.revalidate_publication(&cancel).unwrap();
        assert_ne!(old_identity, new_identity);
        stage.cleanup().unwrap();
        drop(stage);
        let pins: Vec<_> = events.aliases.lock().unwrap().values().cloned().collect();
        let scopes: Vec<_> = pins
            .iter()
            .flat_map(|pin| known(pin.as_ref()).to_vec())
            .collect();
        assert!(
            scopes
                .iter()
                .any(|scope| scope.object == Some(old_identity))
        );
        assert!(
            scopes
                .iter()
                .any(|scope| scope.object == Some(new_identity))
        );
        assert_eq!(
            std::fs::read(temp.path().join("outside/old-alias")).unwrap(),
            b"old bytes"
        );
        assert_eq!(std::fs::read(&target_path).unwrap(), b"replacement bytes");
        assert_eq!(
            events.aliases.lock().unwrap().len(),
            3,
            "target, recovery directory and item observers"
        );
        drop(old_readback);
        drop(new_readback);
        assert!(pins.iter().all(|pin| !known(pin.as_ref()).is_empty()));
    });
}

#[test]
fn retained_native_private_siblings_are_touched_by_exact_arc_without_history_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        events.register(
            parent
                .native_child_namespace_scope(OsStr::new("selected"), EntryKind::Directory, &cancel)
                .unwrap()
                .unwrap(),
        );
        const COUNT: usize = 32;
        let stages: Vec<_> = (0..COUNT)
            .map(|_| {
                let stage = parent.source_stage(&cancel).unwrap();
                let original = events.alias_turns.lock().unwrap().last().unwrap()[0];
                (stage, original)
            })
            .collect();
        assert_eq!(events.aliases.lock().unwrap().len(), COUNT);
        assert!(
            events
                .alias_turns
                .lock()
                .unwrap()
                .iter()
                .all(|aliases| aliases.len() == 1)
        );
        let aliases_before = events.alias_refreshes.load(Ordering::SeqCst);
        let roots_before = events.root_refreshes.load(Ordering::SeqCst);
        for (index, (stage, original)) in stages.into_iter().enumerate() {
            // Production remove_empty_stage must reannounce the already retained
            // private Arc, not reconstruct a matching metadata capability.
            parent.remove_empty_stage(stage).unwrap();
            assert_eq!(
                events.alias_turns.lock().unwrap().last().unwrap(),
                &[original]
            );
            assert_eq!(events.aliases.lock().unwrap().len(), COUNT - index - 1);
        }
        assert!(events.alias_refreshes.load(Ordering::SeqCst) - aliases_before <= 2 * COUNT);
        assert!(events.root_refreshes.load(Ordering::SeqCst) - roots_before <= 2 * COUNT);
    });
}

#[test]
fn failed_native_parent_removal_reannounces_private_descendants_but_not_unrelated_siblings() {
    let temp = tempfile::tempdir().unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        events.register(
            parent
                .native_child_namespace_scope(OsStr::new("selected"), EntryKind::Directory, &cancel)
                .unwrap()
                .unwrap(),
        );
        let outer = parent.source_stage(&cancel).unwrap();
        let nested = outer.source_stage(&cancel).unwrap();
        let unrelated = parent.source_stage(&cancel).unwrap();
        let unrelated_id = events.alias_turns.lock().unwrap().last().unwrap()[0];
        let before = events.alias_refreshes.load(Ordering::SeqCst);
        assert!(parent.remove_empty_stage(outer.clone()).is_err());
        assert!(outer.path().is_dir() && nested.path().is_dir());
        let touched = events.alias_turns.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            touched.len(),
            2,
            "the parent and its retained descendant must both be announced"
        );
        assert!(!touched.contains(&unrelated_id));
        assert_eq!(events.alias_refreshes.load(Ordering::SeqCst) - before, 4);
        assert_eq!(events.aliases.lock().unwrap().len(), 3);
        outer.remove_empty_stage(nested).unwrap();
        parent.remove_empty_stage(outer).unwrap();
        parent.remove_empty_stage(unrelated).unwrap();
        assert!(events.aliases.lock().unwrap().is_empty());
    });
}

#[test]
fn native_named_stage_lifecycles_announce_original_root_automatically_with_linear_refresh_work() {
    let temp = tempfile::tempdir().unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        const COUNT: usize = 256;
        for index in 0..COUNT {
            let pin = parent
                .native_child_namespace_scope(
                    OsStr::new(&format!("selected-{index}")),
                    EntryKind::File,
                    &cancel,
                )
                .unwrap()
                .unwrap();
            events.register(pin);
        }
        for _ in 0..COUNT {
            // These are the production named-stage helpers. The test never
            // calls reserve_private_child to manufacture opt-in.
            let stage = parent.source_stage(&cancel).unwrap();
            assert!(stage.path().is_dir());
            parent.remove_empty_stage(stage).unwrap();
            assert!(events.aliases.lock().unwrap().is_empty());
        }
        assert_eq!(events.turns.lock().unwrap().len(), 2 * COUNT);
        assert!(
            events
                .turns
                .lock()
                .unwrap()
                .iter()
                .all(|roots| !roots.is_empty())
        );
        assert!(events.root_refreshes.load(Ordering::SeqCst) <= 4 * COUNT);
    });
}

#[test]
fn actual_publication_and_directory_creation_report_the_original_materialized_target_arc() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        for (name, kind) in [
            ("sentinel", EntryKind::File),
            ("file", EntryKind::File),
            ("tree", EntryKind::Directory),
        ] {
            events.register(
                parent
                    .native_child_namespace_scope(OsStr::new(name), kind, &cancel)
                    .unwrap()
                    .unwrap(),
            );
        }
        let parent = directory(temp.path());
        let mut stage = parent.stage_file(&cancel).unwrap();
        stage.write_all(b"real native bytes").unwrap();
        assert!(
            events.turns.lock().unwrap().is_empty(),
            "anonymous I/O must stay Stable"
        );
        assert!(matches!(
            stage
                .publish_no_replace(OsStr::new("file"), &cancel)
                .unwrap(),
            TransferPublication::Published(_)
        ));
        stage.cleanup().unwrap();
        let child = parent
            .create_directory(OsStr::new("tree"), &cancel)
            .unwrap();
        let mut nested = child.stage_file(&cancel).unwrap();
        nested.write_all(b"nested native bytes").unwrap();
        assert!(matches!(
            nested
                .publish_no_replace(OsStr::new("child"), &cancel)
                .unwrap(),
            TransferPublication::Published(_)
        ));
        nested.cleanup().unwrap();
        let turns = events.turns.lock().unwrap();
        assert!(turns[0].contains(&1));
        assert!(turns[1].contains(&2));
        assert!(
            turns[2].contains(&2),
            "materialized subtree must remain indexed for descendant changes"
        );
        let roots = events.roots.lock().unwrap();
        assert!(known(roots[1].as_ref())[0].object.is_some());
        assert_eq!(known(roots[2].as_ref())[0].object, Some(child.identity()));
    });
}

#[test]
fn dropped_planning_roots_cannot_leave_a_private_pin_as_the_next_sentinel() {
    let temp = tempfile::tempdir().unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        let old = parent
            .native_child_namespace_scope(OsStr::new("old"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        drop(old);
        let root = parent
            .native_child_namespace_scope(OsStr::new("retry"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        events.register(root);
        let stage = parent.source_stage(&cancel).unwrap();
        parent.remove_empty_stage(stage).unwrap();
        assert!(
            events
                .turns
                .lock()
                .unwrap()
                .iter()
                .all(|roots| roots == &[0])
        );
    });
}

#[test]
fn native_replacement_announces_original_displaced_object_aliases() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("target"), b"original").unwrap();
    std::fs::hard_link(temp.path().join("target"), temp.path().join("alias")).unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        for name in ["sentinel", "target", "alias"] {
            events.register(
                parent
                    .native_child_namespace_scope(OsStr::new(name), EntryKind::File, &cancel)
                    .unwrap()
                    .unwrap(),
            );
        }
        let target = parent
            .prepare_target(OsStr::new("target"), &cancel)
            .unwrap()
            .unwrap();
        let mut stage = parent
            .stage_file_for_target(
                OsStr::new("target"),
                target,
                midnight_core::TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel,
            )
            .unwrap();
        stage.write_all(b"replacement").unwrap();
        assert!(matches!(
            stage
                .publish_replace(OsStr::new("target"), &cancel)
                .unwrap(),
            TransferPublication::Replaced { .. }
        ));
        stage.cleanup().unwrap();
        let turns = events.turns.lock().unwrap();
        assert_eq!(turns.len(), 1);
        assert!(turns[0].contains(&1) && turns[0].contains(&2));
        assert_eq!(
            std::fs::read(temp.path().join("alias")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(temp.path().join("target")).unwrap(),
            b"replacement"
        );
    });
}

#[test]
fn actual_native_move_announces_both_original_endpoints_before_its_private_capture() {
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("source");
    let destination_path = temp.path().join("destination");
    std::fs::write(&source_path, b"native move").unwrap();
    std::fs::create_dir(&destination_path).unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let provider = crate::LinuxFileSystem::new(None);
        let cancel = Cancellation::default();
        let prepared = provider.prepare_move_source(&source_path, &cancel).unwrap();
        let destination = provider
            .prepare_move_destination(&destination_path, &cancel)
            .unwrap();
        events.register(
            prepared
                .source
                .native_namespace_scope(&cancel)
                .unwrap()
                .unwrap(),
        );
        events.register(
            destination
                .native_child_namespace_scope(OsStr::new("moved"), EntryKind::File, &cancel)
                .unwrap()
                .unwrap(),
        );
        let result = prepared
            .capture
            .try_native_move(destination.as_ref(), OsStr::new("moved"), None, &cancel)
            .unwrap();
        assert!(matches!(
            result,
            midnight_core::TransferNativeMoveOutcome::Moved { .. }
        ));
        let turns = events.turns.lock().unwrap();
        assert_eq!(turns.len(), 1);
        assert!(turns[0].contains(&0) && turns[0].contains(&1));
        assert!(events.aliases.lock().unwrap().is_empty());
        assert!(!source_path.exists());
        assert_eq!(
            std::fs::read(destination_path.join("moved")).unwrap(),
            b"native move"
        );
    });
}

#[test]
fn private_only_job_cannot_mint_an_original_root_sentinel() {
    let temp = tempfile::tempdir().unwrap();
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let stage = parent.source_stage(&Cancellation::default()).unwrap();
        parent.remove_empty_stage(stage).unwrap();
        assert_eq!(
            events.turns.lock().unwrap().as_slice(),
            &[Vec::<usize>::new(), Vec::new()]
        );
        assert!(events.aliases.lock().unwrap().is_empty());
    });
}

#[test]
fn foreign_observer_after_native_opt_in_refuses_mutation_and_preserves_the_owned_stage() {
    let temp = tempfile::tempdir().unwrap();
    let foreign = directory(temp.path());
    let events = Arc::new(AnnouncementEvents::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(AnnouncementContext(events.clone()));
    let (parent, stage, failure) = with_transfer_namespace_changes(context, || {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        events.register(
            parent
                .native_child_namespace_scope(OsStr::new("selected"), EntryKind::File, &cancel)
                .unwrap()
                .unwrap(),
        );
        let (stage, failure) = change(&cancel, |mut turn| {
            let stage = parent.source_stage_in(&cancel, &mut turn)?;
            let failure = foreign
                .create_directory_in(OsStr::new("must-not-appear"), &cancel, &mut turn)
                .err();
            // Retain the already-created native owner in the explicit result;
            // failure to announce a later binding must not hide or drop it.
            Ok((stage, failure))
        })
        .unwrap();
        (parent, stage, failure)
    });
    assert!(failure.is_some());
    assert_eq!(events.unknown.load(Ordering::SeqCst), 1);
    assert!(!temp.path().join("must-not-appear").exists());
    assert!(stage.path().is_dir());
    parent.remove_empty_stage(stage).unwrap();
}

fn known(pin: &dyn TransferNamespacePin) -> Arc<[TransferNamespaceScope]> {
    match pin.refresh(&Cancellation::default()).unwrap() {
        TransferNamespaceProbe::Known(facts) => facts,
        TransferNamespaceProbe::Unknown => panic!("fixture requires qualified local byte lookup"),
    }
}

fn directory(path: &Path) -> destination::Directory {
    destination::Directory::from_file(File::open(path).unwrap(), path.to_owned()).unwrap()
}

#[test]
fn distinct_siblings_share_parent_metadata_without_claiming_the_parent_subtree() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let cancel = Cancellation::default();
    let first = parent
        .native_child_namespace_scope(OsStr::new("first"), EntryKind::File, &cancel)
        .unwrap()
        .unwrap();
    let second = parent
        .native_child_namespace_scope(OsStr::new("second"), EntryKind::File, &cancel)
        .unwrap()
        .unwrap();
    let a = known(first.as_ref());
    let b = known(second.as_ref());
    assert!(Arc::ptr_eq(&a[0].parent, &b[0].parent));
    assert_ne!(a[0].lookup_key, b[0].lookup_key);
    assert_eq!(a[0].extent, TransferNamespaceExtent::Entry);
    assert_eq!(a[0].object, None);
    assert_eq!(a[0].parent.ancestors[0], parent.identity());
}

#[test]
fn raw_names_and_hard_link_object_aliases_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let a = OsString::from_vec(vec![b'a', 0xff]);
    let b = OsString::from("A");
    std::fs::write(temp.path().join(&a), b"bytes").unwrap();
    std::fs::hard_link(temp.path().join(&a), temp.path().join(&b)).unwrap();
    let parent = directory(temp.path());
    let cancel = Cancellation::default();
    let first = parent
        .native_child_namespace_scope(&a, EntryKind::File, &cancel)
        .unwrap()
        .unwrap();
    let second = parent
        .native_child_namespace_scope(&b, EntryKind::File, &cancel)
        .unwrap()
        .unwrap();
    let a_facts = known(first.as_ref());
    let b_facts = known(second.as_ref());
    assert_eq!(&*a_facts[0].lookup_key, a.as_bytes());
    assert_ne!(a_facts[0].lookup_key, b_facts[0].lookup_key);
    assert_eq!(a_facts[0].object, b_facts[0].object);
    assert!(a_facts[0].object.is_some());
}

#[test]
fn source_pin_rejects_replacement_and_content_revision_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source");
    std::fs::write(&path, b"original").unwrap();
    let fs = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = fs.prepare_transfer_source(&path, &cancel).unwrap();
    let pin = source.native_namespace_scope(&cancel).unwrap().unwrap();
    drop(source);
    std::fs::write(&path, b"changed length").unwrap();
    assert!(pin.refresh(&cancel).is_err());
    std::fs::rename(&path, temp.path().join("retained")).unwrap();
    std::fs::write(&path, b"original").unwrap();
    assert!(pin.refresh(&cancel).is_err());
}

#[test]
fn moved_parent_with_decoy_does_not_adopt_a_new_binding() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("parent");
    std::fs::create_dir(&path).unwrap();
    let parent = directory(&path);
    let cancel = Cancellation::default();
    let pin = parent
        .native_child_namespace_scope(OsStr::new("item"), EntryKind::File, &cancel)
        .unwrap()
        .unwrap();
    std::fs::rename(&path, temp.path().join("moved")).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(pin.refresh(&cancel).is_err());
}

#[test]
fn owned_directory_creation_materializes_the_exact_prospective_subtree() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let cancel = Cancellation::default();
    let pin = parent
        .native_child_namespace_scope(OsStr::new("new"), EntryKind::Directory, &cancel)
        .unwrap()
        .unwrap();
    let before = known(pin.as_ref());
    assert_eq!(before[0].object, None);
    let child = parent.create_directory(OsStr::new("new"), &cancel).unwrap();
    let after = known(pin.as_ref());
    assert_eq!(after[0].object, Some(child.identity()));
    assert_eq!(after[0].extent, TransferNamespaceExtent::Subtree);
    assert_eq!(before[0].lookup_key, after[0].lookup_key);
    assert_eq!(before[0].parent.identity, after[0].parent.identity);
}

#[test]
fn external_creation_is_not_an_owned_transition() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let cancel = Cancellation::default();
    let pin = parent
        .native_child_namespace_scope(OsStr::new("new"), EntryKind::Directory, &cancel)
        .unwrap()
        .unwrap();
    std::fs::create_dir(temp.path().join("new")).unwrap();
    assert!(pin.refresh(&cancel).is_err());
}

#[test]
fn casefold_and_unqualified_filesystems_never_grant_byte_inequality() {
    assert!(!qualified_lookup(0xef53, Some(0x4000_0000)));
    assert!(!qualified_lookup(0xef53, None));
    assert!(!qualified_lookup(0xef53, Some(0x0000_0800)));
    assert!(!qualified_lookup(0x6969, Some(0)));
    assert!(qualified_lookup(0xef53, Some(0)));
    assert!(!qualified_lookup(0x0102_1994, None));
    assert!(!qualified_lookup(0x0102_1994, Some(0x4000_0000)));
    assert!(!qualified_lookup(0x0102_1994, Some(0x0000_0800)));
    assert!(qualified_lookup(0x0102_1994, Some(0)));
}

#[test]
#[ignore = "Build supplies MIDNIGHT_CASEFOLD_TMPFS_TEST_DIR on an existing casefold-enabled tmpfs; no fixture mounts"]
fn casefold_tmpfs_missing_case_aliases_are_unqualified() {
    let root = std::env::var_os("MIDNIGHT_CASEFOLD_TMPFS_TEST_DIR")
        .expect("pre-provisioned casefold tmpfs directory");
    let temp = tempfile::tempdir_in(root).unwrap();
    let file = File::open(temp.path()).unwrap();
    assert_eq!(
        rustix::fs::fstatfs(&file).unwrap().f_type as u64,
        0x0102_1994
    );
    assert_ne!(
        rustix::fs::ioctl_getflags(&file).unwrap().bits() & 0x4000_0000,
        0,
        "fixture must actually inherit casefold"
    );
    std::fs::write(temp.path().join("lookup-proof"), b"same native slot").unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("LOOKUP-PROOF")).unwrap(),
        b"same native slot"
    );
    let parent = directory(temp.path());
    for name in ["absent", "ABSENT"] {
        assert!(!temp.path().join(name).exists());
        let pin = parent
            .native_child_namespace_scope(
                OsStr::new(name),
                EntryKind::File,
                &Cancellation::default(),
            )
            .unwrap()
            .unwrap();
        assert!(matches!(
            pin.refresh(&Cancellation::default()).unwrap(),
            TransferNamespaceProbe::Unknown
        ));
    }
}

#[test]
fn more_than_4096_live_job_bindings_keep_reopened_directory_transition_identity() {
    let temp = tempfile::tempdir().unwrap();
    let file = Arc::new(File::open(temp.path()).unwrap());
    let cancel = Cancellation::default();
    assert!(
        parent_key(&file, temp.path(), &cancel).unwrap().is_some(),
        "qualified fixture required"
    );
    let mut live = Vec::new();
    for _ in 0..4096 {
        let context: Arc<dyn TransferNamespaceChanges> =
            Arc::new(Changes(Arc::new(Events::default())));
        let bindings = with_transfer_namespace_changes(context.clone(), || {
            Bindings::for_parent(&file, temp.path())
        })
        .unwrap();
        live.push((context, bindings));
    }
    let events = Arc::new(Events::default());
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(events.clone()));
    with_transfer_namespace_changes(context, || {
        let first = directory(temp.path());
        let original = first
            .native_child_namespace_scope(OsStr::new("created"), EntryKind::Directory, &cancel)
            .unwrap()
            .unwrap();
        assert!(known(original.as_ref())[0].object.is_none());
        let reopened = directory(temp.path());
        let child = reopened
            .create_directory(OsStr::new("created"), &cancel)
            .unwrap();
        assert_eq!(known(original.as_ref())[0].object, Some(child.identity()));
    });
    assert_eq!(events.begun.load(Ordering::SeqCst), 1);
    assert_eq!(events.finished.load(Ordering::SeqCst), 1);
    assert_eq!(live.len(), 4096);
}

#[test]
fn final_parent_and_binding_owner_drops_remove_weak_registry_entries() {
    let temp = tempfile::tempdir().unwrap();
    let file = Arc::new(File::open(temp.path()).unwrap());
    let cancel = Cancellation::default();
    let key = parent_key(&file, temp.path(), &cancel)
        .unwrap()
        .expect("qualified fixture");
    let context: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(Arc::new(Events::default())));
    let binding_key = (key.clone(), Arc::as_ptr(&context) as *const () as usize);
    let bindings = with_transfer_namespace_changes(context.clone(), || {
        Bindings::for_parent(&file, temp.path())
    })
    .unwrap();
    let root = bindings
        .pin(
            &file,
            temp.path(),
            OsStr::new("absent"),
            EntryKind::File,
            false,
            &cancel,
        )
        .unwrap();
    assert!(parent_registry().lock().unwrap().contains_key(&key));
    assert!(
        bindings_registry()
            .lock()
            .unwrap()
            .contains_key(&binding_key)
    );
    drop(bindings);
    assert!(
        bindings_registry()
            .lock()
            .unwrap()
            .contains_key(&binding_key),
        "the live root still owns its observer identity"
    );
    drop(root);
    assert!(!parent_registry().lock().unwrap().contains_key(&key));
    assert!(
        !bindings_registry()
            .lock()
            .unwrap()
            .contains_key(&binding_key)
    );
    // The context alone does not keep a native parent or observer set alive.
    assert_eq!(Arc::strong_count(&context), 1);
}

#[test]
#[ignore = "explicit native scale Build lease; requires >4096 live parent descriptors and 100000 source files"]
fn full_parent_registry_still_pools_one_hundred_thousand_sibling_sources() {
    let temp = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let mut other_parents = Vec::new();
    for index in 0..4096 {
        let path = temp.path().join(format!("parent-{index}"));
        std::fs::create_dir(&path).unwrap();
        let parent = directory(&path);
        let pin = parent
            .native_child_namespace_scope(OsStr::new("absent"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        assert_eq!(known(pin.as_ref())[0].parent.identity, parent.identity());
        other_parents.push(pin);
    }
    let path = temp.path().join("siblings");
    std::fs::create_dir(&path).unwrap();
    let descriptors = || {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .ok()
                    .and_then(|entry| std::fs::read_link(entry.path()).ok())
                    .is_some_and(|held| held.starts_with(&path))
            })
            .count()
    };
    let before = descriptors();
    let fs = crate::LinuxFileSystem::new(None);
    let mut roots = Vec::new();
    for index in 0..100_000 {
        let source_path = path.join(index.to_string());
        std::fs::write(&source_path, []).unwrap();
        let source = fs.prepare_transfer_source(&source_path, &cancel).unwrap();
        roots.push(source.native_namespace_scope(&cancel).unwrap().unwrap());
        drop(source);
        if index % 1000 == 0 {
            assert!(descriptors() <= before + 8);
        }
    }
    assert_eq!(roots.len(), 100_000);
    assert_eq!(other_parents.len(), 4096);
    assert!(descriptors() <= before + 8);
    let first = known(roots[0].as_ref());
    let last = known(roots.last().unwrap().as_ref());
    assert!(Arc::ptr_eq(&first[0].parent, &last[0].parent));
    assert_ne!(first[0].object, last[0].object);
    drop(roots);
    assert_eq!(descriptors(), before);
}

#[test]
fn one_hundred_thousand_metadata_pins_do_not_retain_leaf_descriptors() {
    let temp = tempfile::tempdir().unwrap();
    let fs = crate::LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let descriptors = || {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .ok()
                    .and_then(|entry| std::fs::read_link(entry.path()).ok())
                    .is_some_and(|path| path.starts_with(temp.path()))
            })
            .count()
    };
    let before = descriptors();
    let mut pins = Vec::with_capacity(100_000);
    for index in 0..100_000 {
        let path = temp.path().join(format!("leaf-{index}"));
        std::fs::write(&path, []).unwrap();
        let source = fs.prepare_transfer_source(&path, &cancel).unwrap();
        pins.push(source.native_namespace_scope(&cancel).unwrap().unwrap());
        drop(source);
    }
    let after = descriptors();
    assert!(
        after <= before + 8,
        "metadata pins retained {after} descriptors from {before}"
    );
    let first = known(pins[0].as_ref());
    let last = known(pins.last().unwrap().as_ref());
    assert!(Arc::ptr_eq(&first[0].parent, &last[0].parent));
    assert_ne!(first[0].object, last[0].object);
}

#[derive(Default)]
struct Events {
    begun: AtomicUsize,
    finished: AtomicUsize,
    abandoned: AtomicUsize,
    reservations: Mutex<Vec<Vec<u8>>>,
    pins: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    reject_first: bool,
    panic_finish: bool,
    fail_finish: bool,
    cancel_on_begin: bool,
}

struct Changes(Arc<Events>);
struct Change {
    events: Arc<Events>,
    committed: bool,
}
impl TransferNamespaceChanges for Changes {
    fn begin(&self, cancel: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        self.0.begun.fetch_add(1, Ordering::SeqCst);
        if self.0.cancel_on_begin {
            cancel.cancel();
        }
        Ok(Box::new(Change {
            events: self.0.clone(),
            committed: false,
        }))
    }
}
impl TransferNamespaceChange for Change {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        _: &Cancellation,
    ) -> Result<bool, FsError> {
        let pin = pin.unwrap();
        let facts = known(pin.as_ref());
        let mut names = self.events.reservations.lock().unwrap();
        names.push(facts[0].lookup_key.to_vec());
        let accepted = !self.events.reject_first || names.len() != 1;
        if accepted {
            self.events.pins.lock().unwrap().push(pin);
        }
        Ok(accepted)
    }
    fn finish(mut self: Box<Self>) -> Result<(), FsError> {
        if self.events.panic_finish {
            panic!("namespace finish fixture");
        }
        self.committed = true;
        self.events.finished.fetch_add(1, Ordering::SeqCst);
        if self.events.fail_finish {
            Err(invalid(
                Path::new("namespace fixture"),
                "latched namespace refresh failure",
            ))
        } else {
            Ok(())
        }
    }
}
impl Drop for Change {
    fn drop(&mut self) {
        if !self.committed {
            self.events.abandoned.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[test]
fn ordinary_native_error_finishes_the_turn() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    std::fs::create_dir(temp.path().join("exists")).unwrap();
    let events = Arc::new(Events::default());
    let result = with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        parent.create_directory(OsStr::new("exists"), &Cancellation::default())
    });
    assert!(result.is_err());
    assert_eq!(events.begun.load(Ordering::SeqCst), 1);
    assert_eq!(events.finished.load(Ordering::SeqCst), 1);
    assert_eq!(events.abandoned.load(Ordering::SeqCst), 0);
}

#[test]
fn private_name_conflict_retries_before_creation_with_one_turn() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let events = Arc::new(Events {
        reject_first: true,
        ..Events::default()
    });
    let stage = with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        parent.source_stage(&Cancellation::default())
    })
    .unwrap();
    let names = events.reservations.lock().unwrap();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);
    assert!(!temp.path().join(OsStr::from_bytes(&names[0])).exists());
    assert!(stage.path().is_dir());
    drop(names);
    assert_eq!(events.begun.load(Ordering::SeqCst), 1);
    assert_eq!(events.finished.load(Ordering::SeqCst), 1);
    parent.remove_empty_stage(stage).unwrap();
}

#[test]
fn native_mutation_unwind_keeps_the_created_recovery_namespace_visible() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let events = Arc::new(Events {
        panic_finish: true,
        ..Events::default()
    });
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
            parent.create_directory(OsStr::new("retained"), &Cancellation::default())
        })
    }));
    assert!(caught.is_err());
    assert!(temp.path().join("retained").is_dir());
    assert_eq!(events.finished.load(Ordering::SeqCst), 0);
    assert_eq!(events.abandoned.load(Ordering::SeqCst), 1);
}

#[test]
fn rebound_directory_shares_owned_transitions_only_with_the_same_job() {
    let temp = tempfile::tempdir().unwrap();
    let first: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(Arc::new(Events::default())));
    let second: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(Arc::new(Events::default())));
    let cancel = Cancellation::default();
    let pin_a = with_transfer_namespace_changes(first.clone(), || {
        directory(temp.path())
            .native_child_namespace_scope(OsStr::new("new"), EntryKind::Directory, &cancel)
            .unwrap()
            .unwrap()
    });
    let pin_b = with_transfer_namespace_changes(second, || {
        directory(temp.path())
            .native_child_namespace_scope(OsStr::new("new"), EntryKind::Directory, &cancel)
            .unwrap()
            .unwrap()
    });
    let created = with_transfer_namespace_changes(first, || {
        directory(temp.path()).create_directory(OsStr::new("new"), &cancel)
    })
    .unwrap();
    assert_eq!(known(pin_a.as_ref())[0].object, Some(created.identity()));
    assert!(
        pin_b.refresh(&cancel).is_err(),
        "another job must not rebind its original absent slot"
    );
}

#[test]
fn committed_native_result_survives_latched_refresh_failure() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let events = Arc::new(Events {
        fail_finish: true,
        ..Events::default()
    });
    let child = with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        parent.create_directory(OsStr::new("committed"), &Cancellation::default())
    })
    .unwrap();
    assert!(child.path().is_dir());
    assert_eq!(events.finished.load(Ordering::SeqCst), 1);
    assert_eq!(events.abandoned.load(Ordering::SeqCst), 0);
}

#[test]
fn anonymous_stage_data_io_does_not_begin_a_namespace_turn() {
    use std::io::{Read, Seek, SeekFrom, Write};
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let events = Arc::new(Events::default());
    with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        let cancel = Cancellation::default();
        let mut stage = parent.stage_file(&cancel).unwrap();
        stage.write_all(b"verified bytes").unwrap();
        stage.sync(&cancel).unwrap();
        stage.seek(SeekFrom::Start(0)).unwrap();
        let mut readback = Vec::new();
        stage.read_to_end(&mut readback).unwrap();
        assert_eq!(readback, b"verified bytes");
        assert_eq!(events.begun.load(Ordering::SeqCst), 0);
        let published = stage
            .publish_no_replace(OsStr::new("result"), &cancel)
            .unwrap();
        assert!(matches!(
            published,
            midnight_core::TransferPublication::Published(_)
        ));
        assert_eq!(events.begun.load(Ordering::SeqCst), 1);
        assert_eq!(events.finished.load(Ordering::SeqCst), 1);
        stage.cleanup().unwrap();
    });
}

#[test]
fn private_alias_retires_only_after_unique_owned_directory_close_returns() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let events = Arc::new(Events::default());
    let stage = with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        parent.source_stage(&Cancellation::default())
    })
    .unwrap();
    let pin = events.pins.lock().unwrap()[0].clone();
    assert!(known(pin.as_ref())[0].object.is_some());
    parent.remove_empty_stage(stage).unwrap();
    assert!(known(pin.as_ref()).is_empty());
}

#[test]
fn a_retained_stage_clone_never_turns_weak_disappearance_into_a_close_certificate() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let events = Arc::new(Events::default());
    let stage = with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        parent.source_stage(&Cancellation::default())
    })
    .unwrap();
    let retained = stage.clone();
    let pin = events.pins.lock().unwrap()[0].clone();
    parent.remove_empty_stage(stage).unwrap();
    assert!(matches!(
        pin.refresh(&Cancellation::default()).unwrap(),
        TransferNamespaceProbe::Unknown
    ));
    drop(retained);
    assert!(matches!(
        pin.refresh(&Cancellation::default()).unwrap(),
        TransferNamespaceProbe::Unknown
    ));
}

#[test]
fn native_move_updates_source_and_rebound_target_without_nested_turns() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    std::fs::write(&source, b"native rename").unwrap();
    std::fs::create_dir(&target).unwrap();
    let events = Arc::new(Events::default());
    with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        let cancel = Cancellation::default();
        let fs = crate::LinuxFileSystem::new(None);
        let prepared = fs.prepare_move_source(&source, &cancel).unwrap();
        let source_pin = prepared
            .source
            .native_namespace_scope(&cancel)
            .unwrap()
            .unwrap();
        let destination = fs.prepare_move_destination(&target, &cancel).unwrap();
        let target_pin = destination
            .native_child_namespace_scope(OsStr::new("published"), EntryKind::File, &cancel)
            .unwrap()
            .unwrap();
        let original = known(source_pin.as_ref())[0].object;
        let outcome = prepared
            .capture
            .try_native_move(destination.as_ref(), OsStr::new("published"), None, &cancel)
            .unwrap();
        assert!(matches!(
            outcome,
            midnight_core::TransferNativeMoveOutcome::Moved { .. }
        ));
        assert_eq!(known(source_pin.as_ref())[0].object, original);
        assert_eq!(known(target_pin.as_ref())[0].object, original);
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(target.join("published")).unwrap(),
            b"native rename"
        );
    });
    assert_eq!(events.begun.load(Ordering::SeqCst), 1);
    assert_eq!(events.finished.load(Ordering::SeqCst), 1);
}

#[test]
fn symlinked_parent_uses_unknown_geometry_instead_of_path_alias_authority() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let parent = directory(&alias);
    let pin = parent
        .native_child_namespace_scope(
            OsStr::new("item"),
            EntryKind::File,
            &Cancellation::default(),
        )
        .unwrap()
        .unwrap();
    assert!(matches!(
        pin.refresh(&Cancellation::default()).unwrap(),
        TransferNamespaceProbe::Unknown
    ));
}

#[test]
fn subdirectory_bind_mounts_do_not_hide_ancestry_behind_known_geometry() {
    let mounts = b"41 1 8:1 / /home rw - ext4 /dev/root rw\n42 41 8:1 /hidden/subtree /alias rw - ext4 /dev/root rw\n";
    assert!(mount_is_filesystem_root(mounts, 41));
    assert!(!mount_is_filesystem_root(mounts, 42));
    assert!(!mount_is_filesystem_root(mounts, 43));
}

#[test]
fn cancellation_after_turn_acquisition_finishes_without_creating_a_name() {
    let temp = tempfile::tempdir().unwrap();
    let parent = directory(temp.path());
    let events = Arc::new(Events {
        cancel_on_begin: true,
        ..Events::default()
    });
    let result = with_transfer_namespace_changes(Arc::new(Changes(events.clone())), || {
        parent.create_directory(OsStr::new("cancelled"), &Cancellation::default())
    });
    assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::Interrupted));
    assert!(!temp.path().join("cancelled").exists());
    assert_eq!(events.begun.load(Ordering::SeqCst), 1);
    assert_eq!(events.finished.load(Ordering::SeqCst), 1);
    assert_eq!(events.abandoned.load(Ordering::SeqCst), 0);
}
