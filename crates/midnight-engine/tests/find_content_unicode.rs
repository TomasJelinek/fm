//! Unicode whole-word correctness and an explicit repeatable throughput probe.
#[path = "../src/find_content.rs"]
#[allow(dead_code)]
mod content;

use midnight_core::*;
use std::{io, path::PathBuf, time::Instant};

struct Source<'a> {
    entry: FindEntry,
    bytes: &'a [u8],
    offset: usize,
    maximum_read: usize,
}

impl<'a> Source<'a> {
    fn new(bytes: &'a [u8], maximum_read: usize) -> Self {
        Self {
            entry: FindEntry {
                path: PathBuf::from("generated-unicode-content.txt"),
                kind: EntryKind::File,
                metadata: EntryMetadata {
                    size: Some(bytes.len() as u64),
                    ..Default::default()
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [1; 16],
                },
                revision: FindRevision([0; 6]),
            },
            bytes,
            offset: 0,
            maximum_read,
        }
    }
}

impl FindSource for Source<'_> {
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<FindChild>, FsError> {
        Err(find_error(
            &self.entry.path,
            io::ErrorKind::InvalidInput,
            "not a directory",
        ))
    }
    fn read_chunk(&mut self, output: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        let count = output
            .len()
            .min(self.maximum_read)
            .min(self.bytes.len() - self.offset);
        output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

fn query(text: &str) -> FindContentQuery {
    FindContentQuery {
        text: text.into(),
        case_sensitive: false,
        whole_words: true,
    }
}

fn assert_match(
    prepared: &mut content::PreparedContent,
    bytes: &[u8],
    read: usize,
    expected: bool,
) {
    let mut progress = FindProgress::default();
    let result = prepared.matches(
        &mut Source::new(bytes, read),
        FindLimits::default(),
        &mut progress,
        &Cancellation::default(),
    );
    assert!(
        matches!(result, content::ContentMatch::Match(found) if found == expected),
        "expected {expected}, read={read}"
    );
    assert_eq!(progress.content_bytes, bytes.len() as u64);
}

#[test]
fn unicode_whole_words_keep_scalar_boundaries_across_fold_expansion_and_reuse() {
    for (text, body, expected) in [
        ("STRASSE", "Straße", true),
        ("STRASSE", "Straßenbahn", false),
        ("FFI", "ﬃ", true),
        ("F", "ﬃ", false),
        ("FI", "ﬃ", false),
        ("SS", "ß", true),
        ("S", "ß", false),
        ("Σ", "ς", true),
        ("Ι\u{308}\u{301}", "ΐ", true),
        ("𐐀", "𐐨", true),
        ("WORD", "word\u{203f}", false),
        ("WORD", "\u{203f}word", false),
        ("WORD", "word\u{301}", false),
        ("WORD", "🦀word🦀", true),
        ("文档", "中文档", false),
        ("文档", " 文档 ", true),
    ] {
        let mut prepared = content::PreparedContent::new(&query(text)).unwrap();
        for read in [1, 2, 7, 65_536] {
            assert_match(&mut prepared, body.as_bytes(), read, expected);
            assert_match(&mut prepared, b"unrelated text", read, false);
            assert_match(&mut prepared, body.as_bytes(), read, expected);
        }
    }
}

#[test]
fn reused_unicode_matcher_resets_decoding_between_utf16_utf8_and_empty_files() {
    let mut prepared = content::PreparedContent::new(&query("STRASSE")).unwrap();
    for little in [true, false] {
        let mut utf16 = if little {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for word in "文档 Straße 🦀".encode_utf16() {
            utf16.extend(if little {
                word.to_le_bytes()
            } else {
                word.to_be_bytes()
            });
        }
        for read in [1, 3, 65_536] {
            assert_match(&mut prepared, &utf16, read, true);
            assert_match(&mut prepared, b"", read, false);
            assert_match(&mut prepared, "Straßenbahn".as_bytes(), read, false);
            assert_match(&mut prepared, "Straße".as_bytes(), read, true);
        }
    }
}

#[test]
fn reused_unicode_matcher_recovers_after_malformed_utf16_without_retaining_a_match() {
    let mut prepared = content::PreparedContent::new(&query("STRASSE")).unwrap();
    let mut valid = vec![0xff, 0xfe];
    for word in "Straße ".encode_utf16() {
        valid.extend(word.to_le_bytes());
    }
    for tail in [
        vec![1],
        vec![0, 0xd8],
        vec![0, 0xdc],
        vec![0, 0xd8, b'A', 0],
    ] {
        let mut malformed = valid.clone();
        malformed.extend(tail);
        for read in [1, 3, 65_536] {
            let result = prepared.matches(
                &mut Source::new(&malformed, read),
                FindLimits::default(),
                &mut FindProgress::default(),
                &Cancellation::default(),
            );
            assert!(matches!(result, content::ContentMatch::Failed(error)
                if error.kind == io::ErrorKind::InvalidData));
            assert_match(&mut prepared, b"no matching content", read, false);
            assert_match(&mut prepared, "Straße".as_bytes(), read, true);
            assert_match(&mut prepared, &valid, read, true);
        }
    }
}

#[test]
fn reused_unicode_fold_cache_keeps_distinct_scalars_distinct() {
    let mut prepared = content::PreparedContent::new(&query("Ā")).unwrap();
    // Same low nine codepoint bits; repeated replacement must never reuse
    // another scalar's cached fold when these files are searched in sequence.
    for _ in 0..4 {
        assert_match(&mut prepared, "Ā".as_bytes(), 1, true);
        assert_match(&mut prepared, "\u{300}".as_bytes(), 1, false);
        assert_match(&mut prepared, "ā".as_bytes(), 1, true);
        assert_match(&mut prepared, "\u{301}".as_bytes(), 1, false);
    }
}

#[test]
#[ignore = "explicit measured Unicode content scan; allocates bounded generated test text"]
fn unicode_whole_word_throughput_probe() {
    const MINIMUM_BYTES: usize = 8 * 1024 * 1024;
    let unit = "文档 Ελληνικά 日誌 中文 Ωμέγα e\u{301} 🦀 ";
    let prefix = unit.repeat(MINIMUM_BYTES.div_ceil(unit.len()));
    for expected in [false, true] {
        let mut text = prefix.clone();
        if expected {
            text.push_str(" NeedleStraße ");
        }
        let mut prepared = content::PreparedContent::new(&query("NEEDLESTRASSE")).unwrap();
        let started = Instant::now();
        assert_match(&mut prepared, text.as_bytes(), 65_536, expected);
        eprintln!(
            "UNICODE_WHOLE_WORD_PROBE expected={expected} bytes={} elapsed_us={}",
            text.len(),
            started.elapsed().as_micros()
        );
    }
}
