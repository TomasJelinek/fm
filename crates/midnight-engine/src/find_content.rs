//! Strict streaming UTF-8/BOM decoding and literal matching with bounded state.
use midnight_core::*;
use std::{io, path::Path};
use unicase::UniCase;

const READ_BYTES: usize = 64 * 1024;
pub(super) enum ContentMatch {
    Match(bool),
    Limited(FindBudget),
    Failed(FsError),
    Cancelled,
}

pub(super) fn matches(
    source: &mut dyn FindSource,
    query: &FindContentQuery,
    limits: FindLimits,
    progress: &mut FindProgress,
    cancel: &Cancellation,
) -> ContentMatch {
    let path = source.entry().path.clone();
    if query.text.is_empty() || query.text.len() > 4096 {
        return ContentMatch::Failed(find_error(
            &path,
            io::ErrorKind::InvalidInput,
            "Find content query is empty or exceeds its budget.",
        ));
    }
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
    let mut matcher = Matcher::new(query);
    let mut chunk = Vec::with_capacity(READ_BYTES + 3);
    let mut carry = [0; 3];
    let mut carry_len = 0;
    let mut file_bytes = 0u64;
    loop {
        if cancel.is_cancelled() {
            return ContentMatch::Cancelled;
        }
        if expected == Some(file_bytes) {
            break;
        }
        let file_remaining = (limits.max_content_bytes_per_file as u64).saturating_sub(file_bytes);
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
        chunk.resize(carry_len + count, 0);
        chunk[..carry_len].copy_from_slice(&carry[..carry_len]);
        let read = match source.read_chunk(&mut chunk[carry_len..], cancel) {
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
        chunk.truncate(carry_len + read);
        match std::str::from_utf8(&chunk) {
            Ok(text) => {
                matcher.consume(text);
                carry_len = 0;
            }
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                let Ok(text) = std::str::from_utf8(&chunk[..valid]) else {
                    return ContentMatch::Failed(encoding_error(&path));
                };
                matcher.consume(text);
                let tail = &chunk[valid..];
                if tail.len() > carry.len() {
                    return ContentMatch::Failed(encoding_error(&path));
                }
                carry_len = tail.len();
                carry[..carry_len].copy_from_slice(tail);
            }
            Err(_) => return ContentMatch::Failed(encoding_error(&path)),
        }
    }
    if carry_len != 0 {
        return ContentMatch::Failed(encoding_error(&path));
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
    ContentMatch::Match(matcher.found)
}

struct Matcher {
    pattern: Vec<u8>,
    prefix: Vec<usize>,
    matched: usize,
    found: bool,
    at_start: bool,
    case_sensitive: bool,
}
impl Matcher {
    fn new(query: &FindContentQuery) -> Self {
        let pattern = if query.case_sensitive {
            query.text.as_bytes().to_vec()
        } else {
            UniCase::unicode(&query.text).to_folded_case().into_bytes()
        };
        let mut prefix = vec![0; pattern.len()];
        let mut matched = 0;
        for index in 1..pattern.len() {
            while matched != 0 && pattern[index] != pattern[matched] {
                matched = prefix[matched - 1];
            }
            if pattern[index] == pattern[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self {
            pattern,
            prefix,
            matched: 0,
            found: false,
            at_start: true,
            case_sensitive: query.case_sensitive,
        }
    }
    fn consume(&mut self, mut text: &str) {
        if self.at_start && !text.is_empty() {
            self.at_start = false;
            text = text.strip_prefix('\u{feff}').unwrap_or(text);
        }
        // Keep validating later chunks even after a match: malformed trailing
        // content must remain an encoding error instead of becoming a match.
        if self.found || self.pattern.is_empty() {
            return;
        }
        let folded = (!self.case_sensitive).then(|| UniCase::unicode(text).to_folded_case());
        for &byte in folded.as_deref().unwrap_or(text).as_bytes() {
            while self.matched != 0 && byte != self.pattern[self.matched] {
                self.matched = self.prefix[self.matched - 1];
            }
            if byte == self.pattern[self.matched] {
                self.matched += 1;
            }
            if self.matched == self.pattern.len() {
                self.found = true;
                return;
            }
        }
    }
}
fn encoding_error(path: &Path) -> FsError {
    find_error(
        path,
        io::ErrorKind::InvalidData,
        "Content search supports strict UTF-8 only; this file has an unsupported or malformed encoding.",
    )
}
