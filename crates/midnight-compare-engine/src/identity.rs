//! Incremental whole-input evidence. Owned and driven only by the comparison worker.
use crate::{IdentityVerdict, check_cancel};
use midnight_plugin_host::CapturedFile;
use midnight_work_budget::{Gate, storage_admission};
use std::sync::{Arc, atomic::AtomicBool};

const BLOCK_BYTES: u32 = 64 * 1024;
const DECODED_SPANS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentityTurn {
    Advanced,
    Blocked,
    Complete(IdentityVerdict),
}

/// Incremental equality of the complete decoded inputs already retained for
/// rendering. No proof is inferred from the diff algorithm or visible rows.
#[derive(Default)]
pub(crate) struct DecodedText {
    lines: [usize; 2],
    offsets: [usize; 2],
    verdict: Option<bool>,
}

impl DecodedText {
    /// Compare at most 64 KiB and visit at most 256 spans per backend turn.
    pub fn step(&mut self, text: &[Vec<String>; 2]) -> Option<bool> {
        if let Some(verdict) = self.verdict {
            return Some(verdict);
        }
        let mut budget = BLOCK_BYTES as usize;
        for _ in 0..DECODED_SPANS {
            for (side, lines) in text.iter().enumerate() {
                if let Some(line) = lines.get(self.lines[side])
                    && self.offsets[side] == line.len()
                {
                    self.lines[side] += 1;
                    self.offsets[side] = 0;
                }
            }
            let left = text[0].get(self.lines[0]);
            let right = text[1].get(self.lines[1]);
            // Empty retained spans consume a bounded iteration rather than
            // allowing arbitrary cleanup/scanning within one turn.
            if left.is_some_and(|line| line.len() == self.offsets[0])
                || right.is_some_and(|line| line.len() == self.offsets[1])
            {
                continue;
            }
            let (left, right) = match (left, right) {
                (Some(left), Some(right)) => (left.as_bytes(), right.as_bytes()),
                (None, None) => {
                    self.verdict = Some(true);
                    return self.verdict;
                }
                _ => {
                    self.verdict = Some(false);
                    return self.verdict;
                }
            };
            let amount = (left.len() - self.offsets[0])
                .min(right.len() - self.offsets[1])
                .min(budget);
            if left[self.offsets[0]..self.offsets[0] + amount]
                != right[self.offsets[1]..self.offsets[1] + amount]
            {
                self.verdict = Some(false);
                return self.verdict;
            }
            for offset in &mut self.offsets {
                *offset += amount;
            }
            budget -= amount;
            if budget == 0 {
                return None;
            }
        }
        None
    }
}

pub(crate) struct WholeInput {
    revisions: [u64; 2],
    offset: u64,
    decoded_equal: Option<bool>,
    gate: Arc<Gate>,
    confirmed_ssd: bool,
}

impl WholeInput {
    pub fn new(
        sources: &[CapturedFile; 2],
        decoded_equal: Option<bool>,
        confirmed_ssd: bool,
    ) -> Self {
        Self {
            revisions: sources.each_ref().map(CapturedFile::revision),
            offset: 0,
            decoded_equal,
            gate: midnight_work_budget::shared_io(),
            confirmed_ssd,
        }
    }

    /// One non-queuing I/O admission and at most two 64 KiB reads. The caller
    /// processes its bounded input queue between turns and fences publication.
    pub fn step(
        &mut self,
        sources: &[CapturedFile; 2],
        cancel: &AtomicBool,
        is_current: impl Fn() -> bool,
    ) -> Result<IdentityTurn, String> {
        check_current(cancel, &is_current)?;
        if self.revisions != sources.each_ref().map(CapturedFile::revision) {
            return Err("Whole-input verification belongs to a different captured pair.".into());
        }
        let Some(_permit) = self.gate.try_acquire(storage_admission(self.confirmed_ssd)) else {
            return Ok(IdentityTurn::Blocked);
        };
        if self.decoded_equal == Some(false) {
            return self.finish(sources, IdentityVerdict::Different, cancel, &is_current);
        }
        let [left_length, right_length] = sources.each_ref().map(CapturedFile::length);
        if left_length != right_length {
            return self.finish(sources, self.unequal(), cancel, &is_current);
        }
        if self.offset < left_length {
            let amount = (left_length - self.offset).min(u64::from(BLOCK_BYTES)) as u32;
            let left = sources[0]
                .read_at(self.offset, amount)
                .map_err(|error| format!("Compared file changed or could not be read: {error}"))?;
            check_current(cancel, &is_current)?;
            let right = sources[1]
                .read_at(self.offset, amount)
                .map_err(|error| format!("Compared file changed or could not be read: {error}"))?;
            check_current(cancel, &is_current)?;
            if left.len() != amount as usize || right.len() != amount as usize {
                return Err(
                    "Compared file returned a short read during identity verification.".into(),
                );
            }
            if left != right {
                return self.finish(sources, self.unequal(), cancel, &is_current);
            }
            self.offset += u64::from(amount);
        }
        if self.offset == left_length {
            self.finish(
                sources,
                IdentityVerdict::IdenticalFiles,
                cancel,
                &is_current,
            )
        } else {
            Ok(IdentityTurn::Advanced)
        }
    }

    fn unequal(&self) -> IdentityVerdict {
        if self.decoded_equal == Some(true) {
            IdentityVerdict::IdenticalText
        } else {
            IdentityVerdict::Different
        }
    }

    fn finish(
        &self,
        sources: &[CapturedFile; 2],
        verdict: IdentityVerdict,
        cancel: &AtomicBool,
        is_current: &impl Fn() -> bool,
    ) -> Result<IdentityTurn, String> {
        for source in sources {
            check_current(cancel, is_current)?;
            source
                .revalidate()
                .map_err(|error| format!("Compared file changed: {error}"))?;
        }
        check_current(cancel, is_current)?;
        Ok(IdentityTurn::Complete(verdict))
    }
}

fn check_current(cancel: &AtomicBool, is_current: &impl Fn() -> bool) -> Result<(), String> {
    check_cancel(cancel)?;
    if is_current() {
        Ok(())
    } else {
        Err("Comparison was replaced before identity verification completed.".into())
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
