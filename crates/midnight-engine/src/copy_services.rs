//! The finite service owner retained by Copy, separate from the actor service bag.
use crate::{
    copy_nested_retirement::CheckedRetirement, transfer_composition::QualifiedTransferInstallation,
};

pub(crate) struct CopyServices {
    transfer: Option<QualifiedTransferInstallation>,
}
impl CopyServices {
    pub(crate) fn new(transfer: Option<QualifiedTransferInstallation>) -> Self {
        Self { transfer }
    }
    pub(crate) fn transfer(&self) -> Option<&QualifiedTransferInstallation> {
        self.transfer.as_ref()
    }
}
impl CheckedRetirement for CopyServices {
    fn retirement_is_checked(&self) -> bool {
        self.transfer
            .as_ref()
            .is_none_or(|installation| installation.witness().is_some())
    }
}
