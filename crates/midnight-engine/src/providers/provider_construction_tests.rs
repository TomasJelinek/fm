use super::*;
use crate::providers::{MountedFilesystem, OpenedLocation, ProviderEntry};
use midnight_core::{EntryKind, EntryMetadata, NodeToken};
use midnight_plugin_host::{ContentSource, Error};
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug)]
enum Failure {
    ResolveCancel,
    ContentCancel,
    ContentValidation,
}
#[derive(Debug)]
struct Calls {
    failure: Failure,
    refuse_close: bool,
    mounts: AtomicUsize,
    contents: AtomicUsize,
    providers: AtomicUsize,
}
struct Provider(Arc<Calls>);
impl FilesystemProvider for Provider {
    fn resolve(
        &self,
        _: &StoredLocation,
        cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        if matches!(self.0.failure, Failure::ResolveCancel) {
            cancel.cancel();
        }
        Ok(OpenedLocation {
            root: NodeToken::new(b"root").unwrap(),
            mount: Arc::new(Mount(self.0.clone())),
        })
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        if let Some(owner) = Arc::into_inner(self) {
            owner.0.providers.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}
struct Mount(Arc<Calls>);
impl MountedFilesystem for Mount {
    fn list(
        &self,
        _: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        emit(ProviderEntry {
            token: NodeToken::new(b"file").unwrap(),
            raw_name: Arc::from(&b"file"[..]),
            display_name: "file".into(),
            kind: EntryKind::File,
            metadata: EntryMetadata::default(),
        });
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
    fn open_content(
        &self,
        _: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        if matches!(self.0.failure, Failure::ContentCancel) {
            cancel.cancel();
        }
        Ok(Arc::new(Content(self.0.clone())))
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), ProviderError> {
        let Some(owner) = Arc::into_inner(self) else {
            return Ok(());
        };
        owner.0.mounts.fetch_add(1, Ordering::SeqCst);
        if owner.0.refuse_close && matches!(owner.0.failure, Failure::ResolveCancel) {
            std::mem::forget(owner);
            return Err(ProviderError::Failed(
                "controlled mount close refusal".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Content(Arc<Calls>);
impl ContentSource for Content {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "content".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        if matches!(self.0.failure, Failure::ContentValidation) {
            Err(Error("controlled validation failure".into()))
        } else {
            Ok(())
        }
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        panic!("construction does not read contents")
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        let Some(owner) = Arc::into_inner(self) else {
            return Ok(());
        };
        owner.0.contents.fetch_add(1, Ordering::SeqCst);
        if owner.0.refuse_close {
            std::mem::forget(owner);
            return Err(Error("controlled content close refusal".into()));
        }
        Ok(())
    }
}
fn setup(
    failure: Failure,
    refuse_close: bool,
) -> (Arc<ProviderRegistry>, StoredLocation, Arc<Calls>) {
    let registry = Arc::new(ProviderRegistry::default());
    let address = StoredLocation::new(
        ProviderId::new("construction-fixture").unwrap(),
        NonZeroU32::MIN,
        Arc::from(&b"root"[..]),
    );
    let calls = Arc::new(Calls {
        failure,
        refuse_close,
        mounts: AtomicUsize::new(0),
        contents: AtomicUsize::new(0),
        providers: AtomicUsize::new(0),
    });
    registry
        .register(
            address.provider().clone(),
            Arc::new(Provider(calls.clone())),
        )
        .unwrap();
    (registry, address, calls)
}
fn resolve_cancel(refuse_close: bool) {
    let (registry, address, calls) = setup(Failure::ResolveCancel, refuse_close);
    let context = crate::copy_nested_retirement::Context::default();
    let result = context.with(|| registry.resolve(&address, &Cancellation::default()));
    assert!(matches!(result, Err(ProviderError::Cancelled)));
    assert_eq!(calls.mounts.load(Ordering::SeqCst), 1);
    assert_eq!(
        registry.mounts.load(Ordering::SeqCst),
        usize::from(refuse_close)
    );
    assert_eq!(context.is_quarantined(), refuse_close);
    if refuse_close {
        assert!(
            context
                .take_errors()
                .iter()
                .any(|error| error.message.contains("controlled mount close refusal"))
        );
    }
    registry.release_on_worker().unwrap();
    assert_eq!(
        calls.providers.load(Ordering::SeqCst),
        usize::from(!refuse_close)
    );
}
#[test]
fn cancelled_resolve_consumes_returned_mount_before_releasing_accounting() {
    resolve_cancel(false);
}
#[test]
fn cancelled_resolve_keeps_original_error_and_retains_tails_on_close_refusal() {
    resolve_cancel(true);
}

#[test]
fn standalone_construction_reports_original_and_cleanup_failure_without_a_context() {
    let (registry, address, calls) = setup(Failure::ResolveCancel, true);
    let Err(ProviderError::Failed(message)) = registry.resolve(&address, &Cancellation::default())
    else {
        panic!("standalone refusal must have a visible combined diagnostic");
    };
    assert!(message.contains("cancelled"));
    assert!(message.contains("controlled mount close refusal"));
    assert_eq!(calls.mounts.load(Ordering::SeqCst), 1);
    assert_eq!(registry.mounts.load(Ordering::SeqCst), 1);
    registry.release_on_worker().unwrap();
    assert_eq!(calls.providers.load(Ordering::SeqCst), 0);
}

fn content_failure(failure: Failure, refuse_close: bool) {
    let (registry, address, calls) = setup(failure, refuse_close);
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    let file = location
        .list(location.root(), &Cancellation::default())
        .unwrap()
        .remove(0);
    let context = crate::copy_nested_retirement::Context::default();
    let result = context.with(|| location.open_content(&file.node, &Cancellation::default()));
    match failure {
        Failure::ContentCancel => assert!(matches!(result, Err(ProviderError::Cancelled))),
        Failure::ContentValidation => assert!(
            matches!(result, Err(ProviderError::Failed(ref error)) if error.contains("controlled validation failure"))
        ),
        Failure::ResolveCancel => unreachable!(),
    }
    assert_eq!(calls.contents.load(Ordering::SeqCst), 1);
    assert_eq!(context.is_quarantined(), refuse_close);
    if refuse_close {
        assert!(
            context
                .take_errors()
                .iter()
                .any(|error| error.message.contains("controlled content close refusal"))
        );
    }
    location.release_on_worker().unwrap();
    assert_eq!(
        registry.mounts.load(Ordering::SeqCst),
        usize::from(refuse_close)
    );
    assert_eq!(
        calls.mounts.load(Ordering::SeqCst),
        usize::from(!refuse_close)
    );
    registry.release_on_worker().unwrap();
    assert_eq!(
        calls.providers.load(Ordering::SeqCst),
        usize::from(!refuse_close)
    );
}
#[test]
fn cancelled_content_callback_consumes_source_before_lease() {
    content_failure(Failure::ContentCancel, false);
}
#[test]
fn cancelled_content_close_refusal_retains_lease_and_exact_diagnostic() {
    content_failure(Failure::ContentCancel, true);
}
#[test]
fn rejected_content_validation_keeps_last_source_for_checked_release() {
    content_failure(Failure::ContentValidation, false);
}
#[test]
fn validation_and_content_close_failures_are_both_retained() {
    content_failure(Failure::ContentValidation, true);
}
