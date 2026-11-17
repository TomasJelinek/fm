//! Bounded proven top-level boundaries of one session revision.
//!
//! A parse can only discover boundaries after its own start, so a region
//! whose checkpoints are all evicted cannot regain semantic rendering. The set
//! therefore keeps coverage instead of the newest offsets: when full, it drops
//! the checkpoint whose removal leaves the smallest gap, weighted by its
//! distance from the latest viewport anchor. Coverage stays dense where the
//! user reads and becomes gradually sparser with distance. The BOM and the
//! furthest checkpoint are always kept. After reading 10 MiB forward, every
//! gap within 1 MiB of the end stays below the 32 KiB replay distance.
use std::{collections::BTreeSet, ops::RangeInclusive};

const MAX_CHECKPOINTS: usize = 256;
/// Distance at which a gap counts half as much as the same gap at the focus.
const FOCUS_SCALE: u64 = 256 * 1024;

#[derive(Default)]
pub(crate) struct Checkpoints {
    set: BTreeSet<u64>,
    focus: u64,
}

impl Checkpoints {
    pub fn initialize(&mut self, bom: u64) {
        self.set.insert(bom);
        self.focus = bom;
    }

    /// The latest viewport anchor; nearby checkpoints are evicted last.
    pub fn focus(&mut self, anchor: u64) {
        self.focus = anchor;
    }

    pub fn earliest(&self, range: RangeInclusive<u64>) -> Option<u64> {
        self.set.range(range).next().copied()
    }

    pub fn latest(&self, range: RangeInclusive<u64>) -> Option<u64> {
        self.set.range(range).next_back().copied()
    }

    pub fn extend(&mut self, offsets: impl IntoIterator<Item = u64>) {
        self.set.extend(offsets);
        if self.set.len() <= MAX_CHECKPOINTS {
            return;
        }
        let mut values: Vec<u64> = self.set.iter().copied().collect();
        while values.len() > MAX_CHECKPOINTS {
            let victim = self.victim(&values);
            self.set.remove(&values[victim]);
            values.remove(victim);
        }
    }

    /// Index of the inner checkpoint whose removal leaves the smallest
    /// focus-weighted gap: `(next - previous) / (FOCUS_SCALE + distance)`.
    /// `values` has more than [`MAX_CHECKPOINTS`] ascending entries.
    fn victim(&self, values: &[u64]) -> usize {
        let score = |index: usize| {
            let gap = u128::from(values[index + 1] - values[index - 1]);
            let weight = u128::from(FOCUS_SCALE) + u128::from(values[index].abs_diff(self.focus));
            (gap, weight)
        };
        let mut best = 1;
        let (mut best_gap, mut best_weight) = score(1);
        for index in 2..values.len() - 1 {
            let (gap, weight) = score(index);
            // gap / weight < best_gap / best_weight, without division.
            if gap * best_weight < best_gap * weight {
                (best, best_gap, best_weight) = (index, gap, weight);
            }
        }
        best
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.set.len()
    }
}

#[cfg(test)]
#[path = "checkpoints_tests.rs"]
mod tests;
