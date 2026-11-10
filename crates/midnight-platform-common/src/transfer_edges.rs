//! Bounded evidence for the explicitly selected size-and-samples policy.
use midnight_core::TRANSFER_SAMPLE_BYTES;

pub(super) struct Edges {
    length: u64,
    observed: u64,
    samples: [Vec<u8>; 3],
}

impl Edges {
    pub(super) fn new(length: u64) -> Self {
        let ranges = Self::ranges(length);
        Self {
            length,
            observed: 0,
            samples: std::array::from_fn(|index| Vec::with_capacity(ranges[index].1 as usize)),
        }
    }

    fn ranges(length: u64) -> [(u64, u64); 3] {
        let block = TRANSFER_SAMPLE_BYTES as u64;
        if length <= 3 * block {
            [(0, length), (length, 0), (length, 0)]
        } else {
            [
                (0, block),
                ((length - block) / 2, block),
                (length - block, block),
            ]
        }
    }

    /// Capture samples while streaming the source; no extra source read, whole
    /// file buffer, hashing, or repeated tail-buffer shifting.
    pub(super) fn update(&mut self, bytes: &[u8]) {
        let end = self.observed + bytes.len() as u64;
        for ((start, count), sample) in Self::ranges(self.length).into_iter().zip(&mut self.samples)
        {
            let from = start.max(self.observed);
            let to = (start + count).min(end);
            if from < to {
                sample.extend_from_slice(
                    &bytes[(from - self.observed) as usize..(to - self.observed) as usize],
                );
            }
        }
        self.observed = end;
    }

    pub(super) fn checked_length(length: u64) -> u64 {
        length.min(3 * TRANSFER_SAMPLE_BYTES as u64)
    }

    /// Disjoint head, middle, tail; small files are compared in their entirety.
    pub(super) fn range(length: u64, checked: u64, capacity: usize) -> (u64, usize) {
        let mut consumed = 0;
        for (start, count) in Self::ranges(length) {
            if checked < consumed + count {
                let within = checked - consumed;
                return (
                    start + within,
                    (count - within).min(capacity as u64) as usize,
                );
            }
            consumed += count;
        }
        (length, 0)
    }

    pub(super) fn matches(&self, length: u64, checked: u64, bytes: &[u8]) -> bool {
        if length != self.length {
            return false;
        }
        let mut consumed = 0;
        for ((_, count), sample) in Self::ranges(length).into_iter().zip(&self.samples) {
            if checked < consumed + count {
                let within = (checked - consumed) as usize;
                return sample.get(within..within + bytes.len()) == Some(bytes);
            }
            consumed += count;
        }
        checked == consumed && bytes.is_empty()
    }
}
