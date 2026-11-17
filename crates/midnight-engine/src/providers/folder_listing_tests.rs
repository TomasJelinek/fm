use super::*;
use midnight_core::ProviderId;
use std::{
    num::NonZeroU32,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Provider(Arc<dyn MountedFilesystem>);
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
            mount: self.0.clone(),
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

/// A generic provider, whose folder-only listing is its complete listing: more
/// maximum-size file tokens than the token ledger admits, then one folder.
#[derive(Default)]
struct LargeTokens {
    emitted: AtomicUsize,
}

impl LargeTokens {
    const FILES: usize = super::lease::MAX_LIST_BYTES / NodeToken::MAX_BYTES;
}

impl MountedFilesystem for LargeTokens {
    fn list(
        &self,
        _: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        let mut bytes = [0xa5; NodeToken::MAX_BYTES];
        for value in 1..=Self::FILES + 1 {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            self.emitted.fetch_add(1, Ordering::Relaxed);
            bytes[..8].copy_from_slice(&(value as u64).to_le_bytes());
            let kind = if value > Self::FILES {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            if !emit(ProviderEntry {
                token: NodeToken::new(&bytes).unwrap(),
                raw_name: Arc::from(*b"f"),
                display_name: String::new(),
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

/// Reports a folder, ignores the request to stop and continues with a file.
struct IgnoresStop;

impl MountedFilesystem for IgnoresStop {
    fn list(
        &self,
        _: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        for (value, kind) in [(1, EntryKind::Directory), (2, EntryKind::File)] {
            let _ = emit(ProviderEntry {
                token: token(value),
                raw_name: Arc::from(*b"n"),
                display_name: "n".into(),
                kind,
                metadata: EntryMetadata::default(),
            });
        }
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
}

fn fixture() -> (ProviderRegistry, MountedLocation) {
    fixture_with(Arc::new(Mount))
}

fn fixture_with(mount: Arc<dyn MountedFilesystem>) -> (ProviderRegistry, MountedLocation) {
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new("test.folders").unwrap(),
        NonZeroU32::new(1).unwrap(),
        Arc::from([]),
    );
    registry
        .register(address.provider().clone(), Arc::new(Provider(mount)))
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

#[test]
fn presence_probe_finds_a_folder_past_the_row_budget_without_issuing_references() {
    let (_registry, mount) = fixture();
    assert_eq!(
        mount.has_folders(mount.root(), &Cancellation::default()),
        Ok(true)
    );
    let guessed = midnight_core::NodeRef {
        mount: mount.root().mount,
        token: token(50_001),
    };
    assert_eq!(
        mount
            .parent(&guessed, &Cancellation::default())
            .unwrap_err(),
        ProviderError::StaleReference,
        "a probe must not grant references to the folder it found"
    );
}

#[test]
fn presence_probe_charges_inspected_tokens_to_the_token_ledger() {
    let mount = Arc::new(LargeTokens::default());
    let (_registry, location) = fixture_with(mount.clone());
    assert_eq!(
        location.has_folders(location.root(), &Cancellation::default()),
        Err(ProviderError::ResourceLimited)
    );
    let emitted = mount.emitted.load(Ordering::Relaxed);
    assert!(
        emitted < LargeTokens::FILES,
        "the probe retained {emitted} large tokens instead of stopping at the ledger"
    );
}

#[test]
fn presence_probe_answer_survives_a_provider_that_ignores_the_stop() {
    let (_registry, location) = fixture_with(Arc::new(IgnoresStop));
    assert_eq!(
        location.has_folders(location.root(), &Cancellation::default()),
        Ok(true)
    );
}
