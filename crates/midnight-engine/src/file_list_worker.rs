//! Streaming UTF-8 generation to a private, sealed temporary file.
#[cfg(test)]
#[path = "file_list_worker_tests.rs"]
mod tests;
use super::file_list_actor::sizes::Captured;
use super::file_list_records::{captured_record, check, error, selected};
use midnight_core::*;
use midnight_worker_platform::spool::{MAX_IO, PrivateSpoolDirectory, Spool};
use std::{collections::BTreeMap, fmt, io, sync::Arc};

pub(super) struct Text(pub(super) Spool);
impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedFileList")
            .field("bytes", &self.0.length())
            .finish()
    }
}
impl GeneratedTextSource for Text {
    fn length(&self) -> u64 {
        self.0.length()
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(MAX_IO);
        self.0.read_at(offset, &mut output[..count])
    }
}

pub(super) enum Generated {
    Preview {
        text: String,
        items: u64,
        truncated: bool,
    },
    Content {
        source: Arc<Text>,
        items: u64,
    },
}

#[cfg(test)]
fn generate(request: &FileListRequest, cancel: &Cancellation) -> Result<Generated, FsError> {
    generate_with_sizes(request, cancel, &Captured::default())
}

pub(super) fn generate_with_sizes(
    request: &FileListRequest,
    cancel: &Cancellation,
    captured_sizes: &Captured,
) -> Result<Generated, FsError> {
    check(cancel)?;
    if request.template.is_empty() {
        return Err(error("The file list template cannot be empty."));
    }
    let format = FileListTemplate::parse(&request.template).map_err(error)?;
    let environment: BTreeMap<_, _> = format
        .environment_names()
        .map(|name| {
            std::env::var(name)
                .map(|value| (name.to_owned(), value))
                .map_err(|_| {
                    error(format!(
                        "Environment variable {name} is unavailable or not Unicode."
                    ))
                })
        })
        .collect::<Result<_, _>>()?;
    let entries = selected(&request.capture, cancel)?;
    let sizes = captured_sizes.freeze(&entries, cancel)?;
    let sized_record = |entry: &Entry| {
        let mut row = captured_record(entry, &request.capture, &environment, cancel)?;
        if let Some(bytes) = sizes.directory_size(entry) {
            row.size = bytes.to_string();
        }
        Ok::<_, FsError>(row)
    };
    let mut widths = Vec::new();
    for entry in &entries {
        check(cancel)?;
        format
            .measure(&sized_record(entry)?, &mut widths)
            .map_err(error)?;
    }
    let preview = request.destination == FileListDestination::Preview;
    let mut spool = if preview {
        None
    } else {
        let directory = std::env::temp_dir().join("MidnightExplorer-file-list");
        Some(
            PrivateSpoolDirectory::open(&directory)
                .and_then(|root| root.create())
                .map_err(|e| error(e.to_string()))?,
        )
    };
    let mut text = String::new();
    let mut truncated = false;
    for (index, entry) in entries.iter().enumerate() {
        check(cancel)?;
        if preview && (index >= 12 || text.len() >= 16 * 1024) {
            truncated = true;
            break;
        }
        let row = format
            .render(&sized_record(entry)?, &widths)
            .map_err(error)?;
        if let Some(spool) = &mut spool {
            for bytes in row.as_bytes().chunks(MAX_IO) {
                check(cancel)?;
                spool
                    .append(spool.length(), bytes)
                    .map_err(|e| error(e.to_string()))?;
            }
        } else {
            let remaining = (16usize * 1024).saturating_sub(text.len());
            let mut end = row.len().min(remaining);
            while !row.is_char_boundary(end) {
                end -= 1;
            }
            text.push_str(&row[..end]);
            if end < row.len() {
                truncated = true;
                break;
            }
        }
    }
    check(cancel)?;
    let items = entries.len() as u64;
    match spool {
        Some(mut spool) => {
            spool.finish().map_err(|e| error(e.to_string()))?;
            Ok(Generated::Content {
                source: Arc::new(Text(spool)),
                items,
            })
        }
        None => Ok(Generated::Preview {
            text,
            items,
            truncated,
        }),
    }
}

struct Source {
    source: Arc<Text>,
    offset: u64,
    entry: TransferEntry,
}
pub(super) fn transfer(source: Arc<Text>) -> Box<dyn TransferSource> {
    let entry = TransferEntry {
        item: MutationItem {
            path: "file-list.txt".into(),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(source.length()),
                ..Default::default()
            },
        },
        identity: TransferIdentity {
            device: 0,
            object: [0; 16],
        },
    };
    Box::new(Source {
        source,
        offset: 0,
        entry,
    })
}
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        check(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        check(cancel)?;
        let count = self
            .source
            .read_at(self.offset, buffer)
            .map_err(|e| error(e.to_string()))?;
        self.offset += count as u64;
        Ok(count)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Ok(None)
    }
}
