//! Advisory byte work. Retry activity never becomes logical completion credit.
use super::*;
use midnight_core::{DiscoveryScope, DiscoveryState, TransferRemainingWork};

pub(super) struct Budget {
    readback: Option<u64>,
    published: Option<u64>,
    removed: Option<u64>,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            readback: None,
            published: Some(0),
            removed: Some(0),
        }
    }
}
impl Budget {
    pub(super) fn discovery() -> Self {
        Self {
            readback: Some(0),
            ..Self::default()
        }
    }
    pub(super) fn observe(&mut self, item: &MutationItem, policy: TransferVerification) {
        match item.kind {
            EntryKind::File => {
                self.readback = add_file(self.readback, item.metadata.size, policy);
            }
            EntryKind::Directory => {}
            _ => self.readback = None,
        }
    }
    pub(super) fn published(&mut self, length: Option<u64>, policy: TransferVerification) {
        self.published = add_file(self.published, length, policy);
    }
    pub(super) fn removed(&mut self, length: Option<u64>, policy: TransferVerification) {
        self.removed = add_file(self.removed, length, policy);
    }
    pub(super) fn remaining(
        &self,
        progress: &CopyProgress,
        active_proof_credit: Option<u64>,
    ) -> Option<TransferRemainingWork> {
        if progress.discovery.state != DiscoveryState::Complete
            || progress.discovery.scope != DiscoveryScope::Recursive
            || progress.native_entries_moved != 0
            || progress.skipped != 0
            || progress.failed != 0
        {
            return None;
        }
        let write_bytes = progress
            .discovery
            .bytes?
            .checked_sub(progress.completed_bytes)?
            .checked_sub(progress.current_file_copied)?;
        let readback = self.readback?;
        let initial = readback
            .checked_sub(self.published?)?
            .checked_sub(progress.current_file_verified)?;
        let proof = if progress.intent == TransferIntent::Move {
            readback
                .checked_sub(self.removed?)?
                .checked_mul(4)?
                .checked_sub(active_proof_credit?)?
        } else {
            0
        };
        Some(TransferRemainingWork {
            write_bytes,
            verification_bytes: initial.checked_add(proof)?,
        })
    }
}
fn add_file(total: Option<u64>, length: Option<u64>, policy: TransferVerification) -> Option<u64> {
    total?.checked_add(policy.readback_bytes(length?))
}

#[cfg(test)]
#[path = "copy_work_budget_tests.rs"]
mod tests;
