//! Typed host installation is the provenance for the finite audited native graph.
use midnight_core::{AuditedSynchronousTransferService, LocaleService, TransferService};
use std::sync::Arc;

/// Pure host capability, minted only beside a still-concrete audited service.
/// It is independent of native geometry and carries no native/provider owner.
#[derive(Clone, Copy)]
pub(crate) struct TransferRetirementWitness(());

#[derive(Clone)]
pub(crate) struct QualifiedTransferInstallation {
    service: Arc<dyn TransferService>,
    witness: Option<TransferRetirementWitness>,
}
impl QualifiedTransferInstallation {
    pub(crate) fn unqualified(service: Arc<dyn TransferService>) -> Self {
        Self {
            service,
            witness: None,
        }
    }
    pub(crate) fn audited<T: AuditedSynchronousTransferService + 'static>(service: Arc<T>) -> Self {
        Self {
            service,
            witness: Some(TransferRetirementWitness(())),
        }
    }
    pub(crate) fn service(&self) -> Arc<dyn TransferService> {
        self.service.clone()
    }
    pub(crate) fn witness(&self) -> Option<TransferRetirementWitness> {
        self.witness
    }
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
}

#[cfg(test)]
#[path = "transfer_composition_tests.rs"]
mod tests;
