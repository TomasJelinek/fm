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
            // A kept link is staged and checked by its representation, never read back
            // as content bytes; an entry of another kind is never copied.
            EntryKind::Directory
            | EntryKind::Symlink
            | EntryKind::DirectoryLink
            | EntryKind::Other => {}
        }
    }
    /// The read-back total of a discovery that completed while the job already published
    /// and removed files: those counts stay.
    pub(super) fn adopt_readback(&mut self, discovered: &Self) {
        self.readback = discovered.readback;
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
        {
            return None;
        }
        // An approximate count, or one that skipped or failed items leave partly undone,
        // still estimates what remains: never below nothing.
        let approximate =
            progress.discovery.approximate || progress.skipped != 0 || progress.failed != 0;
        let less = |total: u64, done: u64| {
            if approximate {
                Some(total.saturating_sub(done))
            } else {
                total.checked_sub(done)
            }
        };
        let write_bytes = less(
            less(progress.discovery.bytes?, progress.completed_bytes)?,
            progress.current_file_copied,
        )?;
        let readback = self.readback?;
        let initial = less(
            less(readback, self.published?)?,
            progress.current_file_verified,
        )?;
        let proof = if progress.intent == TransferIntent::Move {
            less(
                less(readback, self.removed?)?.checked_mul(4)?,
                active_proof_credit?,
            )?
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
