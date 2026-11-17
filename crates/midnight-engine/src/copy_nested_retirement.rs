//! Copy-local retirement acknowledgement for explicitly qualified owners.
#[cfg(test)]
use crate::retirement::WorkerOwned;
use midnight_core::{ArchiveMount, FsError};
use std::{
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// An explicit semantic contract, not a consequence of `Send` or a wrapper's
/// destructor returning. A true result promises that all owned native work is
/// synchronous, or joins this Copy context before relinquishing ownership.
/// Implementations must account for every nested provider and deferred owner.
/// Shared references need not wait for unrelated owners, but their final owner
/// must implement the same protocol. Unknown implementations remain retained.
pub(crate) trait CheckedRetirement: Send {
    fn retirement_is_checked(&self) -> bool {
        false
    }
}
impl CheckedRetirement for () {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}

#[derive(Clone, Default)]
pub(crate) struct Context(Arc<State>);
#[derive(Default)]
struct State {
    quarantined: AtomicBool,
    errors: Mutex<Vec<FsError>>,
}
thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}
impl Context {
    pub(crate) fn is_quarantined(&self) -> bool {
        self.0.quarantined.load(Ordering::Acquire)
    }
    pub(crate) fn quarantine(&self) {
        self.0.quarantined.store(true, Ordering::Release);
    }
    pub(crate) fn record(&self, error: FsError) {
        self.quarantine();
        let mut errors = self
            .0
            .errors
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if errors.len() < 16 {
            errors.push(error);
        }
    }
    pub(crate) fn take_errors(&self) -> Vec<FsError> {
        std::mem::take(
            &mut *self
                .0
                .errors
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }
    pub(crate) fn with<R>(&self, operation: impl FnOnce() -> R) -> R {
        struct Restore(Option<Context>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|current| {
                    let mut current = current.borrow_mut();
                    if let Some(active) = &*current
                        && active.is_quarantined()
                        && let Some(previous) = &self.0
                    {
                        previous.quarantine();
                        if !Arc::ptr_eq(&active.0, &previous.0) {
                            for error in active.take_errors() {
                                previous.record(error);
                            }
                        }
                    }
                    *current = self.0.take();
                });
            }
        }
        let previous = CURRENT.with(|current| current.replace(Some(self.clone())));
        let _restore = Restore(previous);
        operation()
    }
}
pub(crate) fn current() -> Option<Context> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Observe only the backing source's cached health inside the current Copy
/// boundary. False is not retirement qualification and performs no release.
pub(crate) fn observe_archive(mount: &dyn ArchiveMount) {
    if let Some(context) = current()
        && mount.cleanup_unconfirmed()
    {
        context.record(unconfirmed());
    }
}

pub(crate) fn unconfirmed() -> FsError {
    FsError::from_io(
        "release archive",
        std::path::Path::new(""),
        std::io::Error::other(
            "Archive native retirement is unconfirmed; Copy authority remains retained.",
        ),
    )
}

/// The archive's consuming capability is the proof, including when its concrete
/// native owner lives behind a trait object. Every failed result trips Copy's
/// sticky seal without initiating a panic from a destructor.
pub(crate) fn release_archive(mount: Arc<dyn ArchiveMount>) -> Result<(), FsError> {
    let context = current().unwrap_or_default();
    if context.is_quarantined() {
        std::mem::forget(mount);
        return Err(unconfirmed());
    }
    let mut result = Ok(());
    let returned = context.with(|| checked_step(|| result = mount.release_on_worker()));
    if !returned && result.is_ok() {
        result = Err(unconfirmed());
    }
    if let Err(error) = &result {
        context.record(error.clone());
    }
    result
}

pub(crate) fn provider_error() -> crate::providers::ProviderError {
    crate::providers::ProviderError::Failed(
        "Provider retirement is unconfirmed; Copy authority remains retained.".into(),
    )
}
pub(crate) fn fs_error(error: impl std::fmt::Display) -> FsError {
    FsError::from_io(
        "release provider",
        std::path::Path::new(""),
        std::io::Error::other(error.to_string()),
    )
}

/// Catches an individual explicit bridge, with its remaining owners armed by
/// the concrete caller. This does not qualify an arbitrary payload's destructor.
pub(crate) fn provider_step(
    operation: impl FnOnce() -> Result<(), crate::providers::ProviderError>,
) -> Result<(), crate::providers::ProviderError> {
    let context = current().unwrap_or_default();
    let mut result = Ok(());
    if !context.with(|| checked_step(|| result = operation())) && result.is_ok() {
        result = Err(provider_error());
    }
    if let Err(error) = &result {
        context.record(fs_error(error));
    }
    result
}
pub(crate) fn content_step(
    operation: impl FnOnce() -> Result<(), midnight_plugin_host::Error>,
) -> Result<(), midnight_plugin_host::Error> {
    let context = current().unwrap_or_default();
    let mut result = Ok(());
    if !context.with(|| checked_step(|| result = operation())) && result.is_ok() {
        result = Err(midnight_plugin_host::Error(provider_error().to_string()));
    }
    if let Err(error) = &result {
        context.record(fs_error(error));
    }
    result
}
pub(crate) fn release_provider(
    owner: Arc<dyn crate::providers::FilesystemProvider>,
) -> Result<(), crate::providers::ProviderError> {
    provider_step(|| owner.release_on_worker())
}
pub(crate) fn release_mount(
    owner: Arc<dyn crate::providers::MountedFilesystem>,
) -> Result<(), crate::providers::ProviderError> {
    provider_step(|| owner.release_on_worker())
}
pub(crate) fn release_captured_file(
    owner: midnight_plugin_host::CapturedFile,
) -> Result<(), midnight_plugin_host::Error> {
    content_step(|| owner.release_on_worker())
}
macro_rules! captured_release {
    ($function:ident, $kind:ident) => {
        pub(crate) fn $function(owner: Arc<dyn midnight_core::$kind>) -> Result<(), FsError> {
            let context = current().unwrap_or_default();
            let mut result = Ok(());
            if !context.with(|| checked_step(|| result = owner.release_on_worker()))
                && result.is_ok()
            {
                result = Err(unconfirmed());
            }
            if let Err(error) = &result {
                context.record(error.clone());
            }
            result
        }
    };
}
captured_release!(release_transfer_sources, CapturedTransferSources);
captured_release!(release_clipboard_sources, CapturedClipboardFiles);
captured_release!(release_drop_directory, CapturedDropDirectory);

/// For an audited owner's individual native destruction step. The owner keeps
/// all remaining native/provider/lease fields outside this closure in armed
/// storage; a caught unwind cannot destroy those fields through local cleanup.
pub(crate) fn checked_step(operation: impl FnOnce()) -> bool {
    let Some(context) = current() else {
        std::mem::forget(operation);
        return false;
    };
    if context.is_quarantined() {
        std::mem::forget(operation);
        return false;
    }
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(()) => !context.is_quarantined(),
        Err(payload) => {
            std::mem::forget(payload);
            context.record(unconfirmed());
            false
        }
    }
}

/// Outside Copy the caller retains its existing retirement-domain behavior.
/// Inside Copy the actual value is consumed synchronously while its original
/// retirement lease stays armed. An opaque owner is never queued and mistaken
/// for a completed native destruction.
#[cfg(test)]
pub(crate) fn retire_owned<T: CheckedRetirement + 'static>(
    owner: WorkerOwned<T>,
) -> Option<WorkerOwned<T>> {
    let Some(context) = current() else {
        return Some(owner);
    };
    let (value, lease) = owner.into_parts();
    let mut value = Retained::new(value);
    let mut lease = Retained::new(lease);
    let mut qualified = false;
    if !checked_step(|| qualified = value.get().retirement_is_checked()) || !qualified {
        context.quarantine();
        return None;
    }
    if checked_step(|| drop(value.take())) {
        drop(lease.take());
    }
    None
}

/// Unwind retains every still-owned field; only a successful checked step may
/// take a field into its real destructor.
pub(crate) struct Retained<T>(Option<T>);
impl<T> Retained<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Some(value))
    }
    pub(crate) fn get(&self) -> &T {
        self.0.as_ref().expect("armed native owner")
    }
    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("armed native owner")
    }
    pub(crate) fn take(&mut self) -> T {
        self.0.take().expect("armed native owner")
    }
}
impl<T> Drop for Retained<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            std::mem::forget(value);
        }
    }
}
