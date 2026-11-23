//! The roots that a job's namespace refreshes leave out, one bit per planned root. Root
//! indices are dense (`0..roots`), so a membership test costs O(1), and a probe never copies
//! or scans the whole set: a selection of 100,001 items plans 200,002 roots, about 25 KB.
use std::sync::Arc;

/// A set of a job's planned roots, one bit each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RootSet(Vec<u64>);
const WORD: usize = u64::BITS as usize;
fn bit(root: usize) -> u64 {
    1 << (root % WORD)
}
impl RootSet {
    fn insert(&mut self, root: usize) {
        let word = root / WORD;
        if self.0.len() <= word {
            self.0.resize(word + 1, 0);
        }
        self.0[word] |= bit(root);
    }
    fn remove(&mut self, root: usize) {
        if let Some(word) = self.0.get_mut(root / WORD) {
            *word &= !bit(root);
        }
    }
    fn contains(&self, root: usize) -> bool {
        self.0
            .get(root / WORD)
            .is_some_and(|word| word & bit(root) != 0)
    }
}
impl FromIterator<usize> for RootSet {
    fn from_iter<I: IntoIterator<Item = usize>>(roots: I) -> Self {
        let mut set = Self::default();
        for root in roots {
            set.insert(root);
        }
        set
    }
}

/// The roots of a granted job that its refreshes leave out. Their union is shared with
/// every probe of the job (`ProbeBatch::reserve`), never copied for one, and changes only
/// when a change claims a deferred root (a copy on write while a probe still holds it).
#[derive(Default)]
pub(super) struct Exclusions {
    /// Selected roots that a granted job claimed or reserved at this grant. They are not
    /// claims yet: each becomes one with its first change that no earlier job refuses.
    deferred: RootSet,
    /// Roots the job only reads whose geometry is unknown (a followed link, a provider
    /// without native geometry). They are never written, so they are left unclaimed
    /// instead of making the whole job exclusive; reads through them are unprotected.
    unprotected: RootSet,
    excluded: Arc<RootSet>,
}
impl Exclusions {
    pub(super) fn new(deferred: &[usize], unprotected: &[usize]) -> Self {
        Self {
            deferred: deferred.iter().copied().collect(),
            unprotected: unprotected.iter().copied().collect(),
            excluded: Arc::new(deferred.iter().chain(unprotected).copied().collect()),
        }
    }
    pub(super) fn is_deferred(&self, root: usize) -> bool {
        self.deferred.contains(root)
    }
    /// Whether the job only reads `root` and left it unclaimed (its geometry is unknown).
    pub(super) fn is_unprotected(&self, root: usize) -> bool {
        self.unprotected.contains(root)
    }
    /// A change claimed deferred `root`: the job's refreshes include it from now on.
    pub(super) fn claim(&mut self, root: usize) {
        self.deferred.remove(root);
        if !self.unprotected.contains(root) {
            Arc::make_mut(&mut self.excluded).remove(root);
        }
    }
    /// The excluded roots, for a probe of the job.
    pub(super) fn excluded(&self) -> Arc<RootSet> {
        self.excluded.clone()
    }
    #[cfg(test)]
    pub(super) fn deferred(&self) -> Vec<usize> {
        (0..self.deferred.0.len() * WORD)
            .filter(|root| self.deferred.contains(*root))
            .collect()
    }
}

impl super::Transaction {
    /// Whether this job only reads each of `roots` and left it unclaimed: a Copy source of
    /// unknown geometry, such as a folder on a Windows volume that creates 8.3 names. An
    /// announcement of such a root, which every change of a native adapter makes when the
    /// job's first original pin is that source, adds nothing to the job's facts: it never
    /// makes the change wait or fail as one of unknown geometry would.
    pub(super) fn unprotected(&self, roots: &[usize]) -> Result<bool, midnight_core::FsError> {
        let sequence = self.context.sequence;
        let state = self.context.registry.state.lock();
        let state = state.unwrap_or_else(|error| error.into_inner());
        match state.records.get(&sequence) {
            Some(record)
                if state.turn == Some(sequence)
                    && record.state != super::FactsState::Quarantined =>
            {
                Ok(roots
                    .iter()
                    .all(|root| record.exclusions.is_unprotected(*root)))
            }
            _ => Err(super::unconfirmed()),
        }
    }
}

/// The roots that a probe refreshes, in order: `roots`, or else every one of the job's
/// `planned` roots, without the `excluded` ones.
pub(super) fn probed<'a>(
    roots: Option<&'a [usize]>,
    planned: usize,
    excluded: &'a RootSet,
) -> impl Iterator<Item = usize> + 'a {
    let every = roots.is_none().then_some(0..planned);
    roots
        .into_iter()
        .flatten()
        .copied()
        .chain(every.into_iter().flatten())
        .filter(move |root| !excluded.contains(*root))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filter that this set replaced on 22 November: one list of the deferred and the
    /// unprotected roots, copied for each probe and scanned for each probed root.
    fn listed(
        roots: Option<&[usize]>,
        planned: usize,
        deferred: &[usize],
        unprotected: &[usize],
    ) -> Vec<usize> {
        let excluded: Vec<usize> = deferred.iter().chain(unprotected).copied().collect();
        let excluded = |root: &usize| excluded.contains(root);
        match roots {
            Some(roots) => roots
                .iter()
                .copied()
                .filter(|root| !excluded(root))
                .collect(),
            None => (0..planned).filter(|root| !excluded(root)).collect(),
        }
    }

    /// A xorshift generator: the same sets on every run.
    struct Random(u64);
    impl Random {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }
    }

    /// Sets of roots of `0..planned`: none, all, the first, the last, every other one, those
    /// at both sides of each 64-bit word boundary, and random ones.
    fn sets(planned: usize, random: &mut Random) -> Vec<Vec<usize>> {
        let mut sets = vec![vec![], (0..planned).collect()];
        if planned > 0 {
            sets.push(vec![0]);
            sets.push(vec![planned - 1]);
            sets.push((0..planned).step_by(2).collect());
            sets.push(
                (0..planned)
                    .filter(|root| matches!(root % 64, 0 | 63))
                    .collect(),
            );
            for _ in 0..2 {
                sets.push((0..planned).filter(|_| random.below(3) == 0).collect());
            }
        }
        sets
    }

    /// The excluded roots as bits leave out of each probe exactly the roots that the list
    /// they replaced left out: for a probe of every root and of touched roots (in any order,
    /// and one past the last), after the grant and after changes claim deferred roots, with
    /// deferred and unprotected sets that are empty, full, first, last, around word
    /// boundaries, random or overlapping, at sizes on both sides of 64-bit words.
    #[test]
    fn excluded_bits_leave_out_what_the_excluded_list_left_out() {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        for planned in [0, 1, 2, 63, 64, 65, 127, 128, 129, 200] {
            let sets = sets(planned, &mut random);
            let mut touched = sets.clone();
            for set in &sets {
                // A turn's touched roots come in any order.
                let mut shuffled = set.clone();
                for index in (1..shuffled.len()).rev() {
                    shuffled.swap(index, random.below(index + 1));
                }
                touched.push(shuffled);
            }
            touched.push(vec![planned, planned + 64]);
            for deferred in &sets {
                for unprotected in &sets {
                    let mut deferred = deferred.clone();
                    let mut exclusions = Exclusions::new(&deferred, unprotected);
                    for claims in 0..4 {
                        let excluded = exclusions.excluded();
                        for roots in std::iter::once(None).chain(touched.iter().map(Some)) {
                            let roots = roots.map(Vec::as_slice);
                            assert_eq!(
                                probed(roots, planned, &excluded).collect::<Vec<_>>(),
                                listed(roots, planned, &deferred, unprotected),
                                "{planned} roots, deferred {deferred:?}, unprotected \
                                 {unprotected:?}, probe of {roots:?}, after {claims} claims"
                            );
                        }
                        if deferred.is_empty() {
                            break;
                        }
                        // A change claims the first, then the last, then any deferred root.
                        let index = match claims {
                            0 => 0,
                            1 => deferred.len() - 1,
                            _ => random.below(deferred.len()),
                        };
                        let root = deferred.remove(index);
                        assert!(exclusions.is_deferred(root));
                        exclusions.claim(root);
                        assert!(!exclusions.is_deferred(root));
                        assert_eq!(exclusions.deferred(), deferred);
                    }
                }
            }
        }
    }

    /// A probe that still holds the excluded roots keeps them as they were when it began,
    /// while a claim changes them for the job's later probes.
    #[test]
    fn a_claim_leaves_the_excluded_roots_of_a_running_probe_as_they_were() {
        let mut exclusions = Exclusions::new(&[3, 64], &[5]);
        let running = exclusions.excluded();
        exclusions.claim(64);
        assert_eq!(
            probed(None, 66, &running).collect::<Vec<_>>(),
            (0..66)
                .filter(|root| ![3, 5, 64].contains(root))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            probed(None, 66, &exclusions.excluded()).collect::<Vec<_>>(),
            (0..66)
                .filter(|root| ![3, 5].contains(root))
                .collect::<Vec<_>>()
        );
    }
}
