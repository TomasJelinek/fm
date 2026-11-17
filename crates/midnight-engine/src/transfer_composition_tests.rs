use super::*;
use midnight_core::*;

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
