use super::*;
use midnight_core::ProviderId;
use midnight_plugin_host::CapturedFile;
use std::{num::NonZeroU32, sync::Mutex};

type Events = Arc<Mutex<Vec<&'static str>>>;
struct Owner(Events);
struct Mount(Events);
#[derive(Debug)]
struct Source(Events);

impl Drop for Owner {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("provider");
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("mount");
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("content");
    }
}

fn token(value: u8) -> NodeToken {
    NodeToken::new(&[value]).unwrap()
}

impl FilesystemProvider for Owner {
    fn resolve(
        &self,
        _location: &StoredLocation,
        _cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        Ok(OpenedLocation {
            mount: Arc::new(Mount(self.0.clone())),
            root: token(1),
        })
    }
}
impl MountedFilesystem for Mount {
    fn list(
        &self,
        _directory: &NodeToken,
        _cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        emit(ProviderEntry {
            token: token(2),
            raw_name: Arc::from(&b"file"[..]),
            display_name: "file".into(),
            kind: EntryKind::File,
            metadata: EntryMetadata::default(),
        });
        Ok(())
    }
    fn parent(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
    fn open_content(
        &self,
        _node: &NodeToken,
        _cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        Ok(Arc::new(Source(self.0.clone())))
    }
}
impl ContentSource for Source {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "file".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), midnight_plugin_host::Error> {
        Ok(())
    }
    fn read_at(&self, _offset: u64, _length: u32) -> Result<Vec<u8>, midnight_plugin_host::Error> {
        Ok(Vec::new())
    }
}

#[test]
fn unregister_retains_provider_until_content_and_mount_destructors_finish() {
    let events: Events = Arc::default();
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new("ownership").unwrap(),
        NonZeroU32::new(1).unwrap(),
        Arc::from([]),
    );
    let instance = registry
        .register(address.provider().clone(), Arc::new(Owner(events.clone())))
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let entries = location
        .list(location.root(), &Cancellation::default())
        .unwrap();
    let content = CapturedFile::from_source(
        location
            .open_content(&entries[0].node, &Cancellation::default())
            .unwrap(),
    )
    .unwrap();
    drop(location);
    registry.unregister(address.provider(), instance).unwrap();
    assert!(
        events.lock().unwrap().is_empty(),
        "The provider must outlive retained plugin objects."
    );
    assert!(
        content.revalidate().is_err(),
        "Revoked content may stay allocated but cannot be used."
    );
    drop(content);
    assert_eq!(*events.lock().unwrap(), ["content", "mount", "provider"]);
}
