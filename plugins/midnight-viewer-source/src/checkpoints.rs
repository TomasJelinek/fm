//! Distributed byte coverage plus a fixed recent ring. Two slots remain for recovery.
use crate::syntax::State;
use std::collections::BTreeMap;

const SPARSE: usize = 128;
const HOT: usize = 126;
const INITIAL_SPACING: u64 = 2048;
pub(crate) const MAX_RECORDS: usize = 256;
pub(crate) const BATCH_HOT: usize = 64;
const _: () = assert!(SPARSE + HOT + 2 <= MAX_RECORDS);

pub(crate) struct Checkpoints {
    sparse: BTreeMap<u64, State>,
    spacing: u64,
    recent: [(u64, State); HOT],
    len: usize,
    next: usize,
}

impl Checkpoints {
    #[cfg(test)]
    pub(crate) fn retained_len(&self) -> usize {
        self.sparse.len() + self.len
    }
    pub(crate) fn new() -> Self {
        Self {
            sparse: BTreeMap::new(),
            spacing: INITIAL_SPACING,
            recent: [(0, State::default()); HOT],
            len: 0,
            next: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn get(&self, offset: &u64) -> Option<&State> {
        self.recent[..self.len]
            .iter()
            .find(|(at, _)| at == offset)
            .map(|(_, state)| state)
            .or_else(|| self.sparse.get(offset))
    }

    pub(crate) fn nearest(&self, anchor: u64) -> Option<(u64, State)> {
        let sparse = self
            .sparse
            .range(..=anchor)
            .next_back()
            .map(|(&at, &state)| (at, state));
        self.recent[..self.len]
            .iter()
            .copied()
            .filter(|(at, _)| *at <= anchor)
            .chain(sparse)
            .max_by_key(|(at, _)| *at)
    }

    pub(crate) fn insert(&mut self, offset: u64, state: State) {
        self.insert_sparse(offset, state);
        if let Some((_, old)) = self.recent[..self.len]
            .iter_mut()
            .find(|(at, _)| *at == offset)
        {
            *old = state;
        } else {
            self.recent[self.next] = (offset, state);
            self.next = (self.next + 1) % HOT;
            self.len = (self.len + 1).min(HOT);
        }
    }

    /// Adds a coverage state unless one is already within the current spacing.
    /// A full cover doubles the spacing and keeps an evenly thinned subset, so
    /// earlier regions stay covered instead of being displaced by later ones.
    fn insert_sparse(&mut self, offset: u64, state: State) {
        if let Some(previous) = self.sparse.get_mut(&offset) {
            *previous = state;
            return;
        }
        while !self.crowded(offset) {
            if self.sparse.len() < SPARSE {
                self.sparse.insert(offset, state);
                return;
            }
            self.spacing = self.spacing.saturating_mul(2);
            let spacing = self.spacing;
            let mut kept: Option<u64> = None;
            // The lowest entry is the BOM, which is always kept.
            self.sparse.retain(|&at, _| {
                let keep = kept.is_none_or(|previous| at - previous >= spacing);
                if keep {
                    kept = Some(at);
                }
                keep
            });
        }
    }

    fn crowded(&self, offset: u64) -> bool {
        let before = self.sparse.range(..offset).next_back();
        let after = self.sparse.range(offset..).next();
        before.is_some_and(|(&at, _)| offset - at < self.spacing)
            || after.is_some_and(|(&at, _)| at - offset < self.spacing)
    }
}

// Counted separately from bounded BTreeMap nodes; state growth must remain visible.
const _: () =
    assert!(size_of::<Checkpoints>() + SPARSE * (size_of::<(u64, State)>() + 128) <= 64 * 1024);

/// Stage sparse coverage across a replay as well as its most recent line states.
/// The fixed arrays avoid an allocation per newline on dense input.
pub(crate) struct Batch {
    sparse: [(u64, State); 17],
    sparse_len: usize,
    recent: [(u64, State); BATCH_HOT],
    len: usize,
    next: usize,
}
const _: () = assert!(size_of::<Batch>() <= 8192);

impl Batch {
    #[cfg(test)]
    pub(crate) fn retained_len(&self) -> usize {
        self.sparse_len + self.len
    }
    pub(crate) fn new() -> Self {
        Self {
            sparse: [(0, State::default()); 17],
            sparse_len: 0,
            recent: [(0, State::default()); BATCH_HOT],
            len: 0,
            next: 0,
        }
    }
    pub(crate) fn push(&mut self, checkpoint: (u64, State)) {
        if self.sparse_len < self.sparse.len()
            && (self.sparse_len == 0 || checkpoint.0 - self.sparse[self.sparse_len - 1].0 >= 4096)
        {
            self.sparse[self.sparse_len] = checkpoint;
            self.sparse_len += 1;
        }
        self.recent[self.next] = checkpoint;
        self.next = (self.next + 1) % BATCH_HOT;
        self.len = (self.len + 1).min(BATCH_HOT);
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u64, State)> + '_ {
        let first = if self.len == BATCH_HOT { self.next } else { 0 };
        self.sparse[..self.sparse_len]
            .iter()
            .copied()
            .chain((0..self.len).map(move |at| self.recent[(first + at) % BATCH_HOT]))
    }
}
