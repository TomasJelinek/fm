use super::*;
use midnight_core::{NodeRef, ProviderId};
use std::{collections::VecDeque, num::NonZeroU32, sync::Mutex};

struct Provider(Arc<dyn MountedFilesystem>);
struct ScriptedMount {
    steps: Mutex<VecDeque<(EntryKind, bool)>>,
    contradictory_parent: bool,
}
impl FilesystemProvider for Provider {
    fn resolve(
        &self,
        _location: &StoredLocation,
        _cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: self.0.clone(),
            root: token(1),
        })
    }
}
fn token(value: u8) -> NodeToken {
    NodeToken::new(&[value]).unwrap()
}
impl MountedFilesystem for ScriptedMount {
    fn list(
        &self,
        _directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        let Some((kind, cancel_after)) = self.steps.lock().unwrap().pop_front() else {
            return Ok(());
        };
        emit(ProviderEntry {
            token: token(2),
            raw_name: Arc::from(&b"file"[..]),
            display_name: "file".into(),
            kind,
            metadata: EntryMetadata::default(),
        });
        if cancel_after {
            cancel.cancel();
        }
        Ok(())
    }
    fn parent(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        Ok(Some(token(if self.contradictory_parent { 2 } else { 1 })))
    }
}

fn setup(
    steps: VecDeque<(EntryKind, bool)>,
    contradictory_parent: bool,
) -> (ProviderRegistry, MountedLocation) {
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new("test.contract").unwrap(),
        NonZeroU32::new(1).unwrap(),
        Arc::from([]),
    );
    registry
        .register(
            address.provider().clone(),
            Arc::new(Provider(Arc::new(ScriptedMount {
                steps: Mutex::new(steps),
                contradictory_parent,
            }))),
        )
        .unwrap();
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    (registry, mount)
}

#[test]
fn contradictory_reissuance_and_parent_never_change_an_issued_file() {
    let (_registry, mount) = setup(
        VecDeque::from([(EntryKind::File, false), (EntryKind::Directory, false)]),
        true,
    );
    let row = mount
        .list(mount.root(), &Cancellation::default())
        .unwrap()
        .remove(0);
    assert!(matches!(
        mount.list(mount.root(), &Cancellation::default()),
        Err(ProviderError::InvalidResponse(_))
    ));
    assert!(matches!(
        mount.parent(&row.node, &Cancellation::default()),
        Err(ProviderError::InvalidResponse(_))
    ));
    assert!(matches!(
        mount.list(&row.node, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn cancellation_after_last_row_does_not_publish_partial_references() {
    let (_registry, mount) = setup(VecDeque::from([(EntryKind::File, true)]), false);
    let cancel = Cancellation::default();
    assert!(matches!(
        mount.list(mount.root(), &cancel),
        Err(ProviderError::Cancelled)
    ));
    assert!(
        !cancel.is_cancelled(),
        "The provider's request token must not cancel its caller."
    );
    let guessed = NodeRef {
        mount: mount.root().mount,
        token: token(2),
    };
    assert!(matches!(
        mount.parent(&guessed, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn active_mount_budget_is_released_after_last_consumer_drops() {
    let (registry, first) = setup(VecDeque::new(), false);
    let address = first.address().clone();
    let mut mounts = vec![first];
    for _ in 1..64 {
        mounts.push(
            registry
                .resolve(&address, &Cancellation::default())
                .unwrap(),
        );
    }
    assert!(matches!(
        registry.resolve(&address, &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    ));
    mounts.pop();
    assert!(registry.resolve(&address, &Cancellation::default()).is_ok());
}
