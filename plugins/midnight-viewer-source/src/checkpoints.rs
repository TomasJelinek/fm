//! Distributed byte coverage plus a fixed recent ring. Two slots remain for recovery.
use crate::syntax::State;
use std::collections::BTreeMap;

const SPARSE: usize = 128;
const HOT: usize = 126;
const STRIDE: u64 = 8192;
pub(crate) const MAX_RECORDS: usize = 256;
pub(crate) const BATCH_HOT: usize = 64;
const _: () = assert!(SPARSE + HOT + 2 <= MAX_RECORDS);

pub(crate) struct Checkpoints {
    sparse: BTreeMap<u64, State>,
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
        if let Some(old) = self.sparse.get_mut(&offset) {
            *old = state;
        } else if self
            .sparse
            .range(..=offset)
            .next_back()
            .is_none_or(|(&at, _)| offset - at >= STRIDE)
        {
            if self.sparse.len() == SPARSE {
                // Remove the tightest interior point, preserving origin and distant
                // coverage. Never let an insertion temporarily exceed the record cap.
                let mut before = None;
                let mut current = None;
                let mut candidate = None;
                for &after in self.sparse.keys() {
                    if let (Some(left), Some(at)) = (before, current) {
                        let distance = after - left;
                        if candidate.is_none_or(|(_, best)| distance < best) {
                            candidate = Some((at, distance));
                        }
                    }
                    before = current;
                    current = Some(after);
                }
                if let Some((at, _)) = candidate {
                    self.sparse.remove(&at);
                }
            }
            self.sparse.insert(offset, state);
        }
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
