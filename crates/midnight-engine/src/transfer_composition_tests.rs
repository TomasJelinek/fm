use super::*;
use midnight_core::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Audited;
impl TransferService for Audited {
    fn prepare_transfer_source(
        &self,
        path: &std::path::Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("composition must not prepare a source: {path:?}")
    }
    fn prepare_transfer_destination(
        &self,
        path: &std::path::Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("composition must not prepare a destination: {path:?}")
    }
}
impl AuditedSynchronousTransferService for Audited {}

/// A reviewed locale half that counts its formatting calls. Its fixed text tells
/// engine formatting apart from text an adapter supplied.
#[derive(Default)]
pub(crate) struct CountingLocale {
    calls: AtomicUsize,
}
impl CountingLocale {
    pub(crate) fn text() -> LocalizedDateTime {
        LocalizedDateTime {
            date: "engine date".into(),
            time: "engine time".into(),
        }
    }
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl LocaleService for CountingLocale {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(NumberLocale::default())
    }
    fn format_datetime(&self, _: SystemTime) -> Option<LocalizedDateTime> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Some(Self::text())
    }
}
impl TransferService for CountingLocale {
    fn prepare_transfer_source(
        &self,
        path: &std::path::Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        panic!("a locale installation must not prepare a source: {path:?}")
    }
    fn prepare_transfer_destination(
        &self,
        path: &std::path::Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("a locale installation must not prepare a destination: {path:?}")
    }
}
impl AuditedSynchronousTransferService for CountingLocale {}

fn item(modified: Option<SystemTime>, localized: Option<LocalizedDateTime>) -> MutationItem {
    MutationItem {
        path: "displayed.txt".into(),
        kind: EntryKind::File,
        metadata: EntryMetadata {
            modified,
            localized_modified: localized,
            ..EntryMetadata::default()
        },
    }
}

#[test]
fn erasing_an_audited_concrete_service_does_not_mint_a_host_witness() {
    let concrete = Arc::new(Audited);
    let erased: Arc<dyn TransferService> = concrete;
    let installation = QualifiedTransferInstallation::unqualified(erased.clone());
    assert!(installation.witness().is_none());
    assert!(Arc::ptr_eq(&installation.service(), &erased));
}

#[test]
fn typed_audited_installation_retains_its_exact_service_and_witness() {
    let concrete = Arc::new(Audited);
    let installation = QualifiedTransferInstallation::audited(concrete.clone());
    let erased: Arc<dyn TransferService> = concrete;
    assert!(installation.witness().is_some());
    assert!(Arc::ptr_eq(&installation.service(), &erased));
    assert!(installation.clone().witness().is_some());
}

#[test]
fn audited_transfer_installation_carries_only_its_reviewed_presentation_locale() {
    let locale = Arc::new(CountingLocale::default());
    assert!(
        QualifiedTransferInstallation::audited(Arc::new(Audited))
            .locale()
            .is_none()
    );
    let installation = QualifiedTransferInstallation::audited(Arc::new(Audited))
        .with_locale(Some(QualifiedLocaleInstallation::audited(locale.clone())));
    assert!(installation.witness().is_some());
    let carried = installation.clone().locale().expect("presentation locale");
    let erased: Arc<dyn LocaleService> = locale.clone();
    assert!(Arc::ptr_eq(&carried.service(), &erased));
    assert_eq!(locale.calls(), 0, "composition formats nothing");
}

#[test]
fn displayed_items_format_an_absent_time_once_and_keep_provider_text() {
    let locale = Arc::new(CountingLocale::default());
    let installation = QualifiedLocaleInstallation::audited(locale.clone());
    let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

    let mut raw = item(Some(time), None);
    installation.localize(&mut raw);
    assert_eq!(
        raw.metadata.localized_modified,
        Some(CountingLocale::text())
    );
    assert_eq!(raw.metadata.modified, Some(time), "the sort key stays raw");
    assert_eq!(locale.calls(), 1);
    // Re-displaying an already formatted item formats nothing again.
    installation.localize(&mut raw);
    assert_eq!(locale.calls(), 1);

    // Provider-supplied text wins, as for listings.
    let provided = LocalizedDateTime {
        date: "provider date".into(),
        time: "provider time".into(),
    };
    let mut supplied = item(Some(time), Some(provided.clone()));
    installation.localize(&mut supplied);
    assert_eq!(supplied.metadata.localized_modified, Some(provided));

    // Nothing to format without a timestamp; without a locale the UI keeps its
    // explicit UTC fallback.
    let mut untimed = item(None, None);
    installation.localize(&mut untimed);
    assert_eq!(untimed.metadata.localized_modified, None);
    let mut unlocalized = item(Some(time), None);
    localize_displayed(&mut unlocalized, None);
    assert_eq!(unlocalized.metadata.localized_modified, None);
    assert_eq!(locale.calls(), 1);
}
