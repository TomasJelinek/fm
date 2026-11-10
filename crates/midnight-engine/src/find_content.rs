//! Bounded streaming text search; compiled query/read storage is reused per search.
use midnight_core::*;
use std::{io, path::Path};

#[path = "find_text_decoder.rs"]
mod decoder;
#[path = "find_text_matcher.rs"]
mod matcher;

const READ_BYTES: usize = 64 * 1024;
pub(super) enum ContentMatch {
    Match(bool),
    Limited(FindBudget),
    Failed(FsError),
    Cancelled,
}

pub(super) struct PreparedContent {
    matcher: matcher::Matcher,
    decoder: decoder::Decoder,
    chunk: Vec<u8>,
}

impl PreparedContent {
    pub(super) fn new(query: &FindContentQuery) -> Result<Self, &'static str> {
        if query.text.is_empty() || query.text.len() > 4096 || query.text.contains('\0') {
            return Err("Find content must contain 1 to 4096 UTF-8 bytes without NUL.");
        }
        Ok(Self {
            matcher: matcher::Matcher::new(query),
            decoder: decoder::Decoder::default(),
            chunk: vec![0; READ_BYTES],
        })
    }

    pub(super) fn matches(
        &mut self,
        source: &mut dyn FindSource,
        limits: FindLimits,
        progress: &mut FindProgress,
        cancel: &Cancellation,
    ) -> ContentMatch {
        self.matcher.reset();
        self.decoder.reset();
        let path = source.entry().path.clone();
        let expected = source.entry().metadata.size;
        if expected.is_some_and(|size| size > limits.max_content_bytes_per_file as u64) {
            return ContentMatch::Limited(FindBudget::FileContent);
        }
        if expected.is_some_and(|size| {
            size > limits
                .max_content_bytes
                .saturating_sub(progress.content_bytes)
        }) {
            return ContentMatch::Limited(FindBudget::TotalContent);
        }
        let mut file_bytes = 0u64;
        loop {
            if cancel.is_cancelled() {
                return ContentMatch::Cancelled;
            }
            if expected == Some(file_bytes) {
                break;
            }
            let file_remaining =
                (limits.max_content_bytes_per_file as u64).saturating_sub(file_bytes);
            let total_remaining = limits
                .max_content_bytes
                .saturating_sub(progress.content_bytes);
            if file_remaining == 0 {
                return ContentMatch::Limited(FindBudget::FileContent);
            }
            if total_remaining == 0 {
                return ContentMatch::Limited(FindBudget::TotalContent);
            }
            let count = (READ_BYTES as u64)
                .min(file_remaining)
                .min(total_remaining)
                .min(expected.map_or(u64::MAX, |size| size.saturating_sub(file_bytes)))
                as usize;
            let read = match source.read_chunk(&mut self.chunk[..count], cancel) {
                Ok(read) if read <= count => read,
                Ok(_) => {
                    return ContentMatch::Failed(find_error(
                        &path,
                        io::ErrorKind::InvalidData,
                        "Find provider returned an invalid read count.",
                    ));
                }
                Err(error) => {
                    return if cancel.is_cancelled() {
                        ContentMatch::Cancelled
                    } else {
                        ContentMatch::Failed(error)
                    };
                }
            };
            if read == 0 {
                break;
            }
            file_bytes += read as u64;
            progress.content_bytes += read as u64;
            match self.decoder.push(&self.chunk[..read], false) {
                Ok(text) => self.matcher.consume(text),
                Err(()) => return ContentMatch::Failed(encoding_error(&path)),
            }
        }
        match self.decoder.push(&[], true) {
            Ok(text) => self.matcher.consume(text),
            Err(()) => return ContentMatch::Failed(encoding_error(&path)),
        }
        if let Err(error) = source.revalidate(cancel) {
            return if cancel.is_cancelled() {
                ContentMatch::Cancelled
            } else {
                ContentMatch::Failed(error)
            };
        }
        if expected.is_some_and(|size| size != file_bytes) {
            return ContentMatch::Failed(find_error(
                &path,
                io::ErrorKind::InvalidData,
                "Find file length changed during content reading.",
            ));
        }
        ContentMatch::Match(self.matcher.finish())
    }
}

// Standalone callers/tests; the recursive worker retains PreparedContent instead.
#[allow(dead_code)]
pub(super) fn matches(
    source: &mut dyn FindSource,
    query: &FindContentQuery,
    limits: FindLimits,
    progress: &mut FindProgress,
    cancel: &Cancellation,
) -> ContentMatch {
    match PreparedContent::new(query) {
        Ok(mut prepared) => prepared.matches(source, limits, progress, cancel),
        Err(error) => ContentMatch::Failed(find_error(
            &source.entry().path,
            io::ErrorKind::InvalidInput,
            error,
        )),
    }
}

fn encoding_error(path: &Path) -> FsError {
    find_error(
        path,
        io::ErrorKind::InvalidData,
        "Content search supports UTF-8 and BOM-marked UTF-16; this file has an unsupported or malformed encoding.",
    )
}
