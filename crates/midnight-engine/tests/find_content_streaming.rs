#[path = "../src/find_content.rs"]
mod content;
use midnight_core::*;
use std::{io, path::PathBuf};

struct Source {
    entry: FindEntry,
    length: usize,
    offset: usize,
    maximum_read: usize,
    bytes: Option<Vec<u8>>,
}
impl Source {
    fn new(length: usize, bytes: Option<Vec<u8>>, maximum_read: usize) -> Self {
        Self {
            entry: FindEntry {
                path: PathBuf::from("generated-content.txt"),
                kind: EntryKind::File,
                metadata: EntryMetadata {
                    size: Some(length as u64),
                    ..Default::default()
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [1; 16],
                },
                revision: FindRevision([0; 6]),
            },
            length,
            offset: 0,
            maximum_read,
            bytes,
        }
    }
}
impl FindSource for Source {
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
            "Not a directory",
        ))
    }
    fn read_chunk(&mut self, output: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        assert!(
            output.len() <= 64 * 1024,
            "Content read workspace exceeded 64 KiB"
        );
        let count = output
            .len()
            .min(self.maximum_read)
            .min(self.length - self.offset);
        if let Some(bytes) = &self.bytes {
            output[..count].copy_from_slice(&bytes[self.offset..self.offset + count]);
        } else {
            output[..count].fill(b'x');
            for (index, byte) in output[..count].iter_mut().enumerate() {
                let absolute = self.offset + index;
                if absolute >= self.length - 6 {
                    *byte = b"needle"[absolute - (self.length - 6)];
                }
            }
        }
        self.offset += count;
        Ok(count)
    }
}

#[test]
fn practical_default_content_scan_finds_a_late_match_above_the_old_whole_file_limit() {
    let mut source = Source::new(9 * 1024 * 1024, None, usize::MAX);
    let mut progress = FindProgress::default();
    let result = content::matches(
        &mut source,
        &FindContentQuery {
            text: "needle".into(),
            case_sensitive: true,
            whole_words: false,
        },
        FindLimits::default(),
        &mut progress,
        &Cancellation::default(),
    );
    assert!(
        matches!(result, content::ContentMatch::Match(true)),
        "A 9 MiB file should be searched with bounded streaming state"
    );
    assert_eq!(progress.content_bytes, 9 * 1024 * 1024);
}

#[test]
fn bom_scalars_and_case_folding_work_when_every_read_is_one_byte() {
    let bytes = "\u{feff}prefix Straße 文档 suffix".as_bytes().to_vec();
    let mut source = Source::new(bytes.len(), Some(bytes), 1);
    let result = content::matches(
        &mut source,
        &FindContentQuery {
            text: "STRASSE 文档".into(),
            case_sensitive: false,
            whole_words: false,
        },
        FindLimits::default(),
        &mut FindProgress::default(),
        &Cancellation::default(),
    );
    assert!(matches!(result, content::ContentMatch::Match(true)));
}

#[test]
fn finding_a_match_does_not_hide_malformed_or_incomplete_utf8_later_in_the_file() {
    for suffix in [vec![0xff], vec![0xf0, 0x9f]] {
        let mut bytes = b"needle".to_vec();
        bytes.extend(suffix);
        let mut source = Source::new(bytes.len(), Some(bytes), 1);
        let result = content::matches(
            &mut source,
            &FindContentQuery {
                text: "needle".into(),
                case_sensitive: true,
                whole_words: false,
            },
            FindLimits::default(),
            &mut FindProgress::default(),
            &Cancellation::default(),
        );
        assert!(
            matches!(result, content::ContentMatch::Failed(error) if error.kind == io::ErrorKind::InvalidData)
        );
    }
}

#[test]
fn streaming_content_limits_remain_explicit() {
    let mut source = Source::new(32, None, 1);
    let limits = FindLimits {
        max_content_bytes_per_file: 3,
        ..Default::default()
    };
    let result = content::matches(
        &mut source,
        &FindContentQuery {
            text: "needle".into(),
            case_sensitive: true,
            whole_words: false,
        },
        limits,
        &mut FindProgress::default(),
        &Cancellation::default(),
    );
    assert!(matches!(
        result,
        content::ContentMatch::Limited(FindBudget::FileContent)
    ));
}

fn scan(
    bytes: Vec<u8>,
    text: &str,
    whole_words: bool,
    maximum_read: usize,
) -> content::ContentMatch {
    let mut source = Source::new(bytes.len(), Some(bytes), maximum_read);
    content::matches(
        &mut source,
        &FindContentQuery {
            text: text.into(),
            case_sensitive: false,
            whole_words,
        },
        FindLimits::default(),
        &mut FindProgress::default(),
        &Cancellation::default(),
    )
}

#[test]
fn utf16_endianness_surrogates_and_bom_survive_single_byte_reads() {
    for little in [true, false] {
        for maximum_read in [1, 2, 3, 7, 65_536] {
            let mut bytes = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for word in "prefix Straße 文档 🦀 suffix".encode_utf16() {
                bytes.extend(if little {
                    word.to_le_bytes()
                } else {
                    word.to_be_bytes()
                });
            }
            assert!(
                matches!(
                    scan(bytes, "STRASSE 文档 🦀", false, maximum_read),
                    content::ContentMatch::Match(true)
                ),
                "UTF-16 little={little}, read={maximum_read}"
            );
        }
    }
}

#[test]
fn whole_words_reject_substrings_connectors_combining_marks_and_partial_case_folds() {
    for (body, query, expected) in [
        ("foobar foo_bar", "foo", false),
        ("_word word_", "word", false),
        ("e\u{301}", "e", false),
        ("Straße", "STRASSE", true),
        ("ß", "s", false),
        ("Straßenbahn", "STRASSE", false),
        ("😀WORD!", "word", true),
        ("prefix. foo! suffix", "foo", true),
        ("foo", "foo", true),
        ("a a", "a a", true),
    ] {
        for maximum_read in [1, 3, 65_536] {
            assert!(
                matches!(
                    scan(body.as_bytes().to_vec(), query, true, maximum_read),
                    content::ContentMatch::Match(found) if found == expected
                ),
                "body={body:?}, query={query:?}, read={maximum_read}, expected={expected}"
            );
        }
    }
}

#[test]
fn whole_word_match_retains_both_boundaries_across_the_read_window() {
    for (before, after, expected) in [('x', ' ', false), (' ', 'x', false), (' ', ' ', true)] {
        let mut text = ".".repeat(65_532);
        text.push(before);
        text.push_str("needle");
        text.push(after);
        assert!(matches!(
            scan(text.into_bytes(), "needle", true, 65_536),
            content::ContentMatch::Match(found) if found == expected
        ));
    }
}

#[test]
fn a_utf16_match_does_not_hide_a_malformed_tail() {
    for tail in [
        vec![0x01],
        vec![0x00, 0xd8],
        vec![0x00, 0xdc],
        vec![0x00, 0xd8, 0x41, 0x00],
    ] {
        let mut bytes = vec![0xff, 0xfe];
        for word in "needle".encode_utf16() {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(tail);
        assert!(matches!(
            scan(bytes, "needle", false, 1),
            content::ContentMatch::Failed(_)
        ));
    }
}

#[test]
#[ignore = "explicit measured content scan; generated source is bounded and contains no user data"]
fn content_matcher_throughput_probe() {
    use std::time::Instant;
    let length = 32 * 1024 * 1024;
    for case_sensitive in [true, false] {
        let query = FindContentQuery {
            text: "needle".into(),
            case_sensitive,
            whole_words: false,
        };
        let started = Instant::now();
        for _ in 0..3 {
            let mut source = Source::new(length, None, 65_536);
            assert!(matches!(
                content::matches(
                    &mut source,
                    &query,
                    FindLimits::default(),
                    &mut FindProgress::default(),
                    &Cancellation::default()
                ),
                content::ContentMatch::Match(true)
            ));
        }
        eprintln!(
            "CONTENT_PROBE case_sensitive={case_sensitive} bytes={} elapsed_ms={}",
            length * 3,
            started.elapsed().as_millis()
        );
    }
}
