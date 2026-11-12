//! Bounded presentation samples collected during backend index construction.
use crate::{Entry, EntryKind};
use std::hash::{Hash, Hasher};

const MAX_FORMAT_SAMPLES: usize = 16;
const MAX_SAMPLE_CHARS: usize = 128;

/// One maximum per displayed binary unit; the largest value alone is not the widest text.
#[derive(Clone, Debug, Default)]
pub struct SizeColumnSamples {
    maxima: [Option<u64>; 5],
}
impl SizeColumnSamples {
    pub fn observe(&mut self, bytes: u64) {
        let unit = if bytes >= 1 << 40 {
            4
        } else if bytes >= 1 << 30 {
            3
        } else if bytes >= 1 << 20 {
            2
        } else if bytes >= 1 << 10 {
            1
        } else {
            0
        };
        self.maxima[unit] = self.maxima[unit].max(Some(bytes));
    }
    pub fn maximum(&self) -> Option<u64> {
        self.values().max()
    }
    pub fn values(&self) -> impl Iterator<Item = u64> + '_ {
        self.maxima.iter().copied().flatten()
    }
}

#[derive(Clone, Debug)]
struct FormatSample {
    shape: u64,
    characters: usize,
    text: String,
}

#[derive(Clone, Debug, Default)]
pub struct MetadataColumnSamples {
    pub sizes: SizeColumnSamples,
    pub has_directory: bool,
    dates: Vec<FormatSample>,
    times: Vec<FormatSample>,
    pub utc_fallback: bool,
}
impl MetadataColumnSamples {
    pub(crate) fn observe(&mut self, entry: &Entry) {
        if entry.kind == EntryKind::File
            && let Some(bytes) = entry.metadata.size
        {
            self.sizes.observe(bytes);
        }
        self.has_directory |= matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
        if let Some(localized) = &entry.metadata.localized_modified {
            retain_sample(&mut self.dates, &localized.date);
            retain_sample(&mut self.times, &localized.time);
        } else if entry.metadata.modified.is_some() {
            self.utc_fallback = true;
        }
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        [&self.dates, &self.times]
            .into_iter()
            .fold(0usize, |total, values| {
                values.iter().fold(
                    total.saturating_add(
                        values
                            .capacity()
                            .saturating_mul(std::mem::size_of::<FormatSample>()),
                    ),
                    |total, value| total.saturating_add(value.text.capacity()),
                )
            })
    }
    /// Canonical ASCII digits are expanded into bounded glyph samples by presentation.
    /// Localized punctuation, named months and native numeral scripts are preserved.
    pub fn dates(&self) -> impl Iterator<Item = &str> {
        self.dates.iter().map(|value| value.text.as_str())
    }
    pub fn times(&self) -> impl Iterator<Item = &str> {
        self.times.iter().map(|value| value.text.as_str())
    }
}

fn retain_sample(samples: &mut Vec<FormatSample>, text: &str) {
    // Repeated date/time formats allocate nothing. Numeric variation does not
    // change column width when sorting or scrolling. This hash is only a bounded
    // presentation-cache key, never a filesystem identity or operation authority.
    let mut shape = std::collections::hash_map::DefaultHasher::new();
    let mut characters = 0;
    for ch in text.chars().take(MAX_SAMPLE_CHARS) {
        (if ch.is_ascii_digit() { '0' } else { ch }).hash(&mut shape);
        characters += 1;
    }
    let shape = shape.finish();
    if samples.iter().any(|sample| sample.shape == shape) {
        return;
    }
    if samples.len() == MAX_FORMAT_SAMPLES
        && samples
            .last()
            .is_some_and(|last| characters < last.characters)
    {
        return;
    }
    let text = text
        .chars()
        .take(MAX_SAMPLE_CHARS)
        .map(|ch| if ch.is_ascii_digit() { '0' } else { ch })
        .collect();
    samples.push(FormatSample {
        shape,
        characters,
        text,
    });
    samples.sort_unstable_by(|left, right| {
        right
            .characters
            .cmp(&left.characters)
            .then_with(|| left.text.cmp(&right.text))
    });
    samples.truncate(MAX_FORMAT_SAMPLES);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DirectoryIndex, EntryId, LocalizedDateTime};

    fn entry() -> Entry {
        let mut entry = Entry::new(EntryId(1), "file".into(), EntryKind::File);
        entry.metadata.localized_modified = Some(LocalizedDateTime {
            date: "25. listopadu 2026".into(),
            time: "11:07 PM".into(),
        });
        entry
    }
    #[test]
    fn samples_follow_locale_formats_and_units_without_sort_order_jitter() {
        let mut first = entry();
        first.metadata.size = Some((1 << 30) - 1024);
        let mut second = entry();
        second.id = EntryId(2);
        second.metadata.size = Some(2 << 30);
        second.metadata.localized_modified.as_mut().unwrap().date = "15. září 2026".into();
        let a = DirectoryIndex::from_entries(&[first.clone(), second.clone()]).column_samples;
        let b = DirectoryIndex::from_entries(&[second, first]).column_samples;
        assert_eq!(a.sizes.maximum(), Some(2 << 30));
        assert_eq!(
            a.sizes.values().collect::<Vec<_>>(),
            vec![(1 << 30) - 1024, 2 << 30]
        );
        assert_eq!(a.dates().collect::<Vec<_>>(), b.dates().collect::<Vec<_>>());
        assert!(a.dates().any(|date| date == "00. listopadu 0000"));
        assert!(a.times().any(|time| time == "00:00 PM"));
    }
    #[test]
    fn repeated_formats_are_bounded_for_one_hundred_thousand_entries() {
        let mut samples = MetadataColumnSamples::default();
        let entry = entry();
        let started = std::time::Instant::now();
        for _ in 0..100_000 {
            samples.observe(&entry);
        }
        eprintln!("100000 metadata samples: {:?}", started.elapsed());
        assert_eq!(samples.dates.len(), 1);
        assert_eq!(samples.times.len(), 1);
        assert!(samples.retained_bytes() < 1024);
    }
    #[test]
    fn arbitrary_metadata_keeps_bounded_samples() {
        let mut samples = Vec::new();
        for index in 0..1000 {
            retain_sample(&mut samples, &format!("{}{}", "x".repeat(index), index));
        }
        assert!(samples.len() <= MAX_FORMAT_SAMPLES);
        assert!(
            samples
                .iter()
                .all(|value| value.text.chars().count() <= MAX_SAMPLE_CHARS)
        );
    }
}
