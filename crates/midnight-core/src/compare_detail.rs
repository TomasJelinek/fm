//! Why one compared entry got its mark (`CompareMarks::detail`): compact flags and
//! the few numbers a tooltip needs, packed into whole words per stored entry. The
//! UI words them on demand; no text is stored per entry.
use std::ops::{BitOr, BitOrAssign};

/// What about a pair is known without numbers (`CompareDetail::flags`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CompareFlags(u16);

impl CompareFlags {
    /// The modification times differ beyond the tolerance and count as a difference.
    pub const TIME: Self = Self(1 << 5);
    /// The times are one or two hours apart, give or take the tolerance: usually a
    /// daylight saving shift. Counted only together with `TIME`.
    pub const DST_SHIFT: Self = Self(1 << 6);
    /// `CompareDetail::attributes` are POSIX permission bits, not Windows attributes.
    pub const MODE: Self = Self(1 << 7);
    /// Requested metadata (size, time) is unavailable on either side.
    pub const METADATA: Self = Self(1 << 8);
    /// Could not be listed or read.
    pub const UNREADABLE: Self = Self(1 << 9);
    /// Changed while it was compared.
    pub const CHANGED: Self = Self(1 << 10);
    /// A depth or memory limit of the comparison ended it.
    pub const LIMIT: Self = Self(1 << 11);
    /// The two sides are different kinds of item (file, folder, link): not compared.
    pub const KIND: Self = Self(1 << 12);
    /// Several names differ only in case: they cannot be paired.
    pub const CASE: Self = Self(1 << 13);
    /// Every flag above; the low bits say which numbers are stored.
    const ALL: u16 = 0xffe0;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for CompareFlags {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for CompareFlags {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// A compared folder pair's immediate children that did not match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompareInside {
    /// Same-named pairs other than folders (files, kinds that differ) that differ.
    pub files: u32,
    /// Same-named folder pairs that differ somewhere inside or in their attributes.
    pub folders: u32,
    /// Entries present only on the left, only on the right.
    pub only: [u32; 2],
    /// Pairs that could not be compared completely.
    pub unverified: u32,
}

impl CompareInside {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// What differed for one stored entry, left then right where it has two values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompareDetail {
    pub flags: CompareFlags,
    /// The sizes, when they differ.
    pub sizes: Option<[u64; 2]>,
    /// Modification times in whole seconds since the Unix epoch, when they differ
    /// beyond the tolerance or by a daylight saving shift.
    pub times: Option<[i64; 2]>,
    /// The compared attribute bits, when they differ (`CompareFlags::MODE`: POSIX
    /// permissions).
    pub attributes: Option<[u32; 2]>,
    /// Compare contents: the offset of the first differing byte.
    pub first_difference: Option<u64>,
    /// A folder pair: its immediate children that did not match.
    pub inside: Option<CompareInside>,
}

const SIZES: u16 = 1;
const TIMES: u16 = 1 << 1;
const ATTRIBUTES: u16 = 1 << 2;
const CONTENT: u16 = 1 << 3;
const INSIDE: u16 = 1 << 4;

impl CompareDetail {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// `other`'s numbers where it has them, and both sets of flags.
    pub fn merged(mut self, other: &Self) -> Self {
        self.flags |= other.flags;
        self.sizes = other.sizes.or(self.sizes);
        self.times = other.times.or(self.times);
        self.attributes = other.attributes.or(self.attributes);
        self.first_difference = other.first_difference.or(self.first_difference);
        self.inside = other.inside.or(self.inside);
        self
    }

    /// The flags with the low bits telling which numbers follow, and those numbers.
    pub(crate) fn pack(&self, words: &mut Vec<u64>) -> u16 {
        let mut bits = self.flags.0 & CompareFlags::ALL;
        if let Some([a, b]) = self.sizes {
            bits |= SIZES;
            words.extend([a, b]);
        }
        if let Some([a, b]) = self.times {
            bits |= TIMES;
            words.extend([a as u64, b as u64]);
        }
        if let Some([a, b]) = self.attributes {
            bits |= ATTRIBUTES;
            words.push(pair(a, b));
        }
        if let Some(offset) = self.first_difference {
            bits |= CONTENT;
            words.push(offset);
        }
        if let Some(inside) = self.inside {
            bits |= INSIDE;
            words.extend([
                pair(inside.files, inside.folders),
                pair(inside.only[0], inside.only[1]),
                u64::from(inside.unverified),
            ]);
        }
        bits
    }

    /// The flags of `bits` with the numbers `old` announces: a detail whose new
    /// numbers did not fit keeps the ones stored before.
    pub(crate) fn with_numbers_of(bits: u16, old: u16) -> u16 {
        (bits & CompareFlags::ALL) | (old & !CompareFlags::ALL)
    }

    /// How many words `pack` wrote for `bits`.
    pub(crate) fn words(bits: u16) -> usize {
        [
            (SIZES, 2),
            (TIMES, 2),
            (ATTRIBUTES, 1),
            (CONTENT, 1),
            (INSIDE, 3),
        ]
        .into_iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, count)| count)
        .sum()
    }

    /// Reverses `pack`: `words` holds exactly the numbers `bits` announces.
    pub(crate) fn unpack(bits: u16, words: &[u64]) -> Self {
        let mut words = words.iter().copied();
        let mut next = || words.next().unwrap_or_default();
        let mut detail = Self {
            flags: CompareFlags(bits & CompareFlags::ALL),
            ..Self::default()
        };
        if bits & SIZES != 0 {
            detail.sizes = Some([next(), next()]);
        }
        if bits & TIMES != 0 {
            detail.times = Some([next() as i64, next() as i64]);
        }
        if bits & ATTRIBUTES != 0 {
            detail.attributes = Some(split(next()));
        }
        if bits & CONTENT != 0 {
            detail.first_difference = Some(next());
        }
        if bits & INSIDE != 0 {
            let [files, folders] = split(next());
            let only = split(next());
            let unverified = next() as u32;
            detail.inside = Some(CompareInside {
                files,
                folders,
                only,
                unverified,
            });
        }
        detail
    }
}

fn pair(a: u32, b: u32) -> u64 {
    (u64::from(a) << 32) | u64::from(b)
}

fn split(word: u64) -> [u32; 2] {
    [(word >> 32) as u32, word as u32]
}
