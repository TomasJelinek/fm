use crate::{
    casefold::FOLD,
    decode::{self, Encoding},
};
use midnight_plugin_sdk::api::viewer::SourceRange;
use std::collections::VecDeque;

/// Decoded units between host cancellation polls; each poll crosses the C ABI and
/// takes host locks, while 4096 units are microseconds of matching.
const POLL_UNITS: usize = 4096;

pub struct Matcher {
    query: Vec<char>,
    failure: Vec<usize>,
    matched: usize,
    sensitive: bool,
    /// Source start of the unit of each of the last `query.len()` folded characters.
    starts: VecDeque<u64>,
}

/// The match a scan reports.
#[derive(Clone, Copy, Debug)]
pub enum Want {
    /// The first match starting at or after this offset; the scan stops at it.
    FirstFrom(u64),
    /// The last match ending at or before this offset.
    LastUntil(u64),
}

#[derive(Debug)]
pub struct Scan {
    pub found: Option<SourceRange>,
    /// Bytes decoded; an incomplete trailing unit is left for the next read.
    pub consumed: usize,
}

impl Matcher {
    pub fn new(query: &str, sensitive: bool) -> Self {
        let query: Vec<_> = query.chars().flat_map(|c| fold(c, sensitive)).collect();
        let mut failure = vec![0; query.len()];
        for i in 1..query.len() {
            let mut n = failure[i - 1];
            while n > 0 && query[n] != query[i] {
                n = failure[n - 1];
            }
            if query[n] == query[i] {
                n += 1;
            }
            failure[i] = n;
        }
        Self {
            starts: VecDeque::with_capacity(query.len()),
            query,
            failure,
            matched: 0,
            sensitive,
        }
    }

    /// Largest source span of one match: each folded query character belongs to a
    /// decoded unit of at most four bytes.
    pub fn span_bound(&self) -> u64 {
        4 * self.query.len() as u64
    }

    /// Forgets a partial match before scanning a window that does not continue the
    /// previous one.
    pub fn reset(&mut self) {
        self.matched = 0;
        self.starts.clear();
    }

    /// Decodes `bytes` (starting at source `offset`) and feeds every displayed
    /// character, polling `cancelled` every POLL_UNITS units. Partial matches carry
    /// over to the next call, which must continue at `offset + consumed`.
    pub fn scan(
        &mut self,
        bytes: &[u8],
        encoding: Encoding,
        offset: u64,
        eof: bool,
        want: Want,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Scan, ()> {
        let mut found = None;
        let mut consumed = 0;
        for (index, (unit, end)) in decode::units(bytes, encoding, eof).enumerate() {
            if index.is_multiple_of(POLL_UNITS) && cancelled() {
                return Err(());
            }
            let (Some(start), Some(source_end)) = (
                offset.checked_add(consumed as u64),
                offset.checked_add(end as u64),
            ) else {
                break;
            };
            let (characters, count) = unit.visible();
            for &character in &characters[..count] {
                for folded in fold(character, self.sensitive) {
                    let Some(range) = self.step(folded, start, source_end) else {
                        continue;
                    };
                    match want {
                        Want::FirstFrom(from) if range.offset >= from => {
                            return Ok(Scan {
                                found: Some(range),
                                consumed: end,
                            });
                        }
                        Want::LastUntil(until) if source_end <= until => found = Some(range),
                        _ => {}
                    }
                }
            }
            consumed = end;
        }
        Ok(Scan { found, consumed })
    }

    fn step(&mut self, c: char, start: u64, end: u64) -> Option<SourceRange> {
        if self.query.is_empty() {
            return None;
        }
        if self.starts.len() == self.query.len() {
            self.starts.pop_front();
        }
        self.starts.push_back(start);
        while self.matched > 0 && self.query[self.matched] != c {
            self.matched = self.failure[self.matched - 1];
        }
        if self.query[self.matched] == c {
            self.matched += 1;
        }
        if self.matched < self.query.len() {
            return None;
        }
        self.matched = self.failure[self.matched - 1];
        let offset = self.starts.front().copied().unwrap_or(start);
        Some(SourceRange {
            offset,
            length: end - offset,
        })
    }
}

/// Unicode default full case folding (C+F) of one character, without allocating.
/// ASCII folds only A-Z, so it skips the table.
pub(crate) fn fold(c: char, sensitive: bool) -> Folded {
    if sensitive {
        return Folded::One(Some(c));
    }
    if c.is_ascii() {
        return Folded::One(Some(c.to_ascii_lowercase()));
    }
    match FOLD.binary_search_by_key(&c, |entry| entry.0) {
        Ok(index) => Folded::Many(FOLD[index].1.chars()),
        Err(_) => Folded::One(Some(c)),
    }
}

pub(crate) enum Folded {
    One(Option<char>),
    Many(std::str::Chars<'static>),
}

impl Iterator for Folded {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self {
            Self::One(c) => c.take(),
            Self::Many(chars) => chars.next(),
        }
    }
}
