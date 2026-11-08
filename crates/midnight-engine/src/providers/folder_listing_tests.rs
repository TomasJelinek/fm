use super::*;
use midnight_core::ProviderId;
use std::num::NonZeroU32;

struct Provider;
struct Mount;

fn token(value: u64) -> NodeToken {
    NodeToken::new(&value.to_le_bytes()).unwrap()
}

impl FilesystemProvider for Provider {
    fn resolve(
        &self,
        _: &StoredLocation,
        _: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: Arc::new(Mount),
            root: token(0),
        })
    }
}

impl MountedFilesystem for Mount {
    fn list(
        &self,
        _: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        for value in 1..=50_003 {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            let kind = match value {
                50_001 => EntryKind::Directory,
                50_002 => EntryKind::DirectoryLink,
                _ => EntryKind::File,
            };
            if !emit(ProviderEntry {
                token: token(value),
                raw_name: Arc::from([0xff, 0xfe]),
                display_name: format!("name {value}"),
                kind,
                metadata: EntryMetadata::default(),
            }) {
                break;
            }
        }
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
}

fn fixture() -> (ProviderRegistry, MountedLocation) {
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new("test.folders").unwrap(),
        NonZeroU32::new(1).unwrap(),
        Arc::from([]),
    );
    registry
        .register(address.provider().clone(), Arc::new(Provider))
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    (registry, location)
}

#[test]
fn folder_enumeration_does_not_fill_the_result_budget_with_regular_files() {
    let (_registry, mount) = fixture();
    let rows = mount
        .list_folders(mount.root(), &Cancellation::default(), 8)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].entry.kind, EntryKind::Directory);
    assert_eq!(rows[1].entry.kind, EntryKind::DirectoryLink);
    assert_eq!(rows[0].entry.raw_name.as_ref(), &[0xff, 0xfe]);
    assert_eq!(
        mount
            .list(mount.root(), &Cancellation::default())
            .unwrap_err(),
        ProviderError::ResourceLimited
    );
}

#[test]
fn overflow_and_cancel_never_publish_partial_folder_references() {
    let (_registry, mount) = fixture();
    assert_eq!(
        mount
            .list_folders(mount.root(), &Cancellation::default(), 1)
            .unwrap_err(),
        ProviderError::ResourceLimited
    );
    let guessed = midnight_core::NodeRef {
        mount: mount.root().mount,
        token: token(50_001),
    };
    assert_eq!(
        mount
            .parent(&guessed, &Cancellation::default())
            .unwrap_err(),
        ProviderError::StaleReference
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        mount.list_folders(mount.root(), &cancel, 8).unwrap_err(),
        ProviderError::Cancelled
    );
}
