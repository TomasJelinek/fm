//! Literal, bounded archive suffix policy. Native paths are never rewritten.
use crate::{ARCHIVE_ASSOCIATIONS, ArchiveFormat, UserPreferences};
use std::{collections::BTreeMap, path::Path};
use unicase::UniCase;

pub const MAX_ARCHIVE_CUSTOM_EXTENSIONS: usize = 256;
/// Settings resource bound, not a filename or native path length restriction.
pub const MAX_ARCHIVE_EXTENSION_BYTES: usize = 256;

/// Normalize a literal suffix; a single optional leading dot is accepted.
/// Unicode case folding does not perform Unicode normalization or alter native identities.
pub fn normalize_archive_extension(input: &str) -> Result<String, &'static str> {
    let input = input.trim();
    let input = input.strip_prefix('.').unwrap_or(input);
    if input.is_empty() || input.split('.').any(str::is_empty) {
        return Err("Enter an extension with no empty dot-separated parts.");
    }
    if input.len() > MAX_ARCHIVE_EXTENSION_BYTES {
        return Err("An archive extension must fit within 256 UTF-8 bytes.");
    }
    if input.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || (c.is_ascii() && !c.is_ascii_alphanumeric() && !"._-+".contains(c))
    }) {
        return Err("Use a literal extension: no spaces, paths, wildcards or command syntax.");
    }
    let folded = UniCase::new(input).to_folded_case();
    if folded.len() > MAX_ARCHIVE_EXTENSION_BYTES {
        return Err("An archive extension must fit within 256 UTF-8 bytes after case folding.");
    }
    Ok(folded)
}

/// Validate already canonical settings, including conflicts with disabled built-in suffixes.
pub fn validate_archive_custom_extensions(
    extensions: &BTreeMap<String, ArchiveFormat>,
) -> Result<(), &'static str> {
    if extensions.len() > MAX_ARCHIVE_CUSTOM_EXTENSIONS {
        return Err("At most 256 custom archive extensions can be stored.");
    }
    for extension in extensions.keys() {
        if normalize_archive_extension(extension).as_deref() != Ok(extension.as_str()) {
            return Err("Custom archive extensions must use canonical literal suffixes.");
        }
        if ARCHIVE_ASSOCIATIONS
            .iter()
            .any(|known| known.extension == extension)
        {
            return Err("A built-in archive extension must use its existing checkbox.");
        }
    }
    Ok(())
}

/// Backend-prepared case-folded suffixes, shared by UI views without touching files.
/// A valid Unicode suffix can follow a non-Unicode native basename. Only a bounded
/// tail of suffix candidates is retained; the original path is untouched.
#[derive(Clone, Debug, Default)]
pub struct ArchiveExtensionHints {
    suffix: Option<String>,
}

impl ArchiveExtensionHints {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.suffix.as_ref().map_or(0, String::capacity)
    }

    pub fn from_path(path: &Path) -> Self {
        let Some(name) = path.file_name() else {
            return Self::default();
        };
        let bytes = name.as_encoded_bytes();
        // A folded ASCII letter can come from a wider Unicode character (for
        // example Kelvin sign → k). Four UTF-8 bytes per retained folded byte
        // keeps case-equivalent native suffixes matchable without an unbounded scan.
        let start = bytes
            .len()
            .saturating_sub(MAX_ARCHIVE_EXTENSION_BYTES * 4 + 1);
        let suffix = bytes[start..]
            .iter()
            .enumerate()
            .find_map(|(offset, byte)| {
                let dot = start + offset;
                if *byte != b'.' || dot + 1 == bytes.len() {
                    return None;
                }
                let text = std::str::from_utf8(&bytes[dot + 1..]).ok()?;
                // A single hidden-file basename is not an extension.
                if dot == 0 && !text.contains('.') {
                    return None;
                }
                Some(UniCase::new(text).to_folded_case())
            });
        Self { suffix }
    }

    /// Resolve longest suffix first, including disabled built-ins as terminal choices.
    /// This performs bounded literal lookups only and does not check decoder availability.
    pub fn enabled_format(&self, preferences: &UserPreferences) -> Option<ArchiveFormat> {
        let mut suffix = self.suffix.as_deref()?;
        loop {
            if let Some(format) = preferences.archive_custom_extensions.get(suffix) {
                return Some(*format);
            }
            if let Some(known) = ARCHIVE_ASSOCIATIONS
                .iter()
                .find(|known| known.extension == suffix)
            {
                return preferences
                    .archive_association_enabled(suffix)
                    .then_some(known.format);
            }
            suffix = suffix.split_once('.')?.1;
        }
    }
}
