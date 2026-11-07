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
