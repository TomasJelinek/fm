//! Explicit native sizing does not become a fallback for missing installed providers.
use super::*;
use crate::{Request, Response, Transport};
use midnight_core::{SizeDirectoryIdentity, SizeDirectoryTarget};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Resolver(AtomicUsize);
impl DirectoryService for Resolver {
    fn resolve_size_directory(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Some(SizeDirectoryTarget {
            path: path.into(),
            identity: SizeDirectoryIdentity {
                device: 5,
                object: [7; 16],
            },
        }))
    }
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        panic!("native sizing resolution does not replace installed listing")
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("native sizing resolution does not replace installed metadata")
    }
}

#[derive(Debug)]
struct UnusedTransport;
impl Transport for UnusedTransport {
    fn call(&self, _: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        panic!("identity service does not change the installed provider ABI")
    }
}

#[test]
fn size_identity_service_requires_an_available_installed_provider() {
    let installed = InstalledProvider::default();
    installed
        .provider
        .set(Err(invalid("missing provider")))
        .unwrap();
    let resolver = Arc::new(Resolver::default());
    assert!(installed.set_size_directory_service(resolver.clone()));
    assert!(
        installed
            .resolve_size_directory(Path::new("native"), &Cancellation::default())
            .is_err()
    );
    assert_eq!(resolver.0.load(Ordering::SeqCst), 0);
}

#[test]
fn size_identity_service_is_explicit_and_never_replaced_after_installation() {
    let installed = InstalledProvider::default();
    installed
        .provider
        .set(Ok(NativeProvider {
            transport: Arc::new(UnusedTransport),
            capabilities: midnight_filesystem_plugin_api::abi::CAP_NATIVE_ADDRESS,
        }))
        .unwrap();
    assert_eq!(
        installed
            .resolve_size_directory(Path::new("native"), &Cancellation::default())
            .unwrap_err()
            .kind,
        std::io::ErrorKind::Unsupported
    );
    let resolver = Arc::new(Resolver::default());
    assert!(installed.set_size_directory_service(resolver.clone()));
    assert!(!installed.set_size_directory_service(Arc::new(Resolver::default())));
    let target = installed
        .resolve_size_directory(Path::new("native"), &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(target.identity.device, 5);
    assert_eq!(resolver.0.load(Ordering::SeqCst), 1);
}

#[test]
fn cancelled_size_identity_resolution_does_not_discover_provider() {
    let installed = InstalledProvider::default();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        installed
            .resolve_size_directory(Path::new("native"), &cancel)
            .unwrap_err()
            .kind,
        std::io::ErrorKind::Interrupted
    );
    assert!(installed.provider.get().is_none());
}
