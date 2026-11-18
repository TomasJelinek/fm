//! Typed host installation is the provenance for the finite audited native graph.
use midnight_core::{
    AuditedSynchronousTransferService, LocaleService, MutationItem, TransferService,
};
use std::sync::Arc;

/// Pure host capability, minted only beside a still-concrete audited service.
/// It is independent of native geometry and carries no native/provider owner.
#[derive(Clone, Copy)]
pub(crate) struct TransferRetirementWitness(());

#[derive(Clone)]
pub(crate) struct QualifiedTransferInstallation {
    service: Arc<dyn TransferService>,
    witness: Option<TransferRetirementWitness>,
    locale: Option<QualifiedLocaleInstallation>,
}
impl QualifiedTransferInstallation {
    /// Test fixtures only: production installs transfers through
    /// `Engine::configure_audited_transfers`, which always records the audited witness.
    #[cfg(test)]
    pub(crate) fn unqualified(service: Arc<dyn TransferService>) -> Self {
        Self {
            service,
            witness: None,
            locale: None,
        }
    }
    pub(crate) fn audited<T: AuditedSynchronousTransferService + 'static>(service: Arc<T>) -> Self {
        Self {
            service,
            witness: Some(TransferRetirementWitness(())),
            locale: None,
        }
    }
    /// The reviewed locale that formats the entries a transfer prompt displays.
    /// It is audited like the service, so a job retaining it stays checked.
    pub(crate) fn with_locale(mut self, locale: Option<QualifiedLocaleInstallation>) -> Self {
        self.locale = locale;
        self
    }
    pub(crate) fn service(&self) -> Arc<dyn TransferService> {
        self.service.clone()
    }
    pub(crate) fn witness(&self) -> Option<TransferRetirementWitness> {
        self.witness
    }
    pub(crate) fn locale(&self) -> Option<QualifiedLocaleInstallation> {
        self.locale.clone()
    }
}

/// A transfer adapter may report raw modification times only, as both native
/// adapters do. Only an entry shown to the user, such as either side of a conflict
/// prompt, is formatted, and only when its provider left the text absent: the listing
/// rule (`localize_metadata`). The engine never formats enumerated, inspected or
/// automatically resolved entries. Call on a backend worker.
pub(crate) fn localize_displayed(item: &mut MutationItem, locale: Option<&dyn LocaleService>) {
    crate::localize_metadata(&mut item.metadata, locale);
}

/// The reviewed local object supplies locale without an unrelated erased owner.
#[derive(Clone)]
pub(crate) struct QualifiedLocaleInstallation {
    service: Arc<dyn LocaleService>,
}
impl QualifiedLocaleInstallation {
    pub(crate) fn audited<T: LocaleService + AuditedSynchronousTransferService + 'static>(
        service: Arc<T>,
    ) -> Self {
        Self { service }
    }
    pub(crate) fn service(&self) -> Arc<dyn LocaleService> {
        self.service.clone()
    }
    /// `localize_displayed` through this reviewed locale.
    pub(crate) fn localize(&self, item: &mut MutationItem) {
        localize_displayed(item, Some(self.service.as_ref()));
    }
}

#[cfg(test)]
#[path = "transfer_composition_tests.rs"]
pub(crate) mod tests;
