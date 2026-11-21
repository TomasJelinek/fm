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
/// Every file row of a listing holds one, so a suffix of up to
/// [`INLINE_SUFFIX_BYTES`] bytes (nearly all real ones) is stored inline: the hint
/// is 16 bytes and allocates nothing.
#[derive(Clone, Default)]
pub struct ArchiveExtensionHints {
    suffix: Suffix,
}

/// The longest folded suffix stored without an allocation.
const INLINE_SUFFIX_BYTES: usize = 14;

#[derive(Clone, Default)]
enum Suffix {
    #[default]
    None,
    /// The first `length` bytes of `bytes` are the folded suffix, valid UTF-8.
    Inline {
        length: u8,
        bytes: [u8; INLINE_SUFFIX_BYTES],
    },
    /// A longer suffix, behind a thin pointer that keeps the hint at 16 bytes.
    Heap(Box<Box<str>>),
}

const _: () = assert!(std::mem::size_of::<ArchiveExtensionHints>() <= 16);

impl Suffix {
    fn new(folded: &str) -> Self {
        match u8::try_from(folded.len()) {
            Ok(length) if folded.len() <= INLINE_SUFFIX_BYTES => {
                let mut bytes = [0; INLINE_SUFFIX_BYTES];
                bytes[..folded.len()].copy_from_slice(folded.as_bytes());
                Self::Inline { length, bytes }
            }
            _ => Self::Heap(Box::new(folded.into())),
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::None => None,
            // Always valid: `new` copies a whole `str`.
            Self::Inline { length, bytes } => {
                std::str::from_utf8(&bytes[..usize::from(*length)]).ok()
            }
            Self::Heap(text) => Some(text),
        }
    }
}

/// `UniCase` folding without a temporary string for the common short ASCII suffix:
/// `UniCase` folds ASCII text to its ASCII lowercase.
fn folded_suffix(text: &str) -> Suffix {
    if !text.is_ascii() || text.len() > INLINE_SUFFIX_BYTES {
        return Suffix::new(&UniCase::new(text).to_folded_case());
    }
    let mut suffix = Suffix::new(text);
    if let Suffix::Inline { length, bytes } = &mut suffix {
        bytes[..usize::from(*length)].make_ascii_lowercase();
    }
    suffix
}

impl std::fmt::Debug for ArchiveExtensionHints {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArchiveExtensionHints")
            .field("suffix", &self.suffix.as_str())
            .finish()
    }
}

impl ArchiveExtensionHints {
    /// Heap bytes beyond the hint itself: none for an inline suffix.
    pub(crate) fn retained_bytes(&self) -> usize {
        match &self.suffix {
            Suffix::Heap(text) => std::mem::size_of::<Box<str>>() + text.len(),
            Suffix::None | Suffix::Inline { .. } => 0,
        }
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
                Some(folded_suffix(text))
            });
        Self {
            suffix: suffix.unwrap_or_default(),
        }
    }

    /// Resolve longest suffix first, including disabled built-ins as terminal choices.
    /// This performs bounded literal lookups only and does not check decoder availability.
    pub fn enabled_format(&self, preferences: &UserPreferences) -> Option<ArchiveFormat> {
        let mut suffix = self.suffix.as_str()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(name: &str) -> ArchiveExtensionHints {
        ArchiveExtensionHints::from_path(Path::new(name))
    }

    #[test]
    fn short_suffixes_stay_inline_and_longer_ones_keep_their_text() {
        assert_eq!(std::mem::size_of::<ArchiveExtensionHints>(), 16);
        for length in 1..=40 {
            let suffix = "x".repeat(length);
            let hint = hints(&format!("name.{suffix}"));
            assert_eq!(hint.suffix.as_str(), Some(suffix.as_str()));
            let expected = if length <= INLINE_SUFFIX_BYTES {
                0
            } else {
                std::mem::size_of::<Box<str>>() + length
            };
            assert_eq!(hint.retained_bytes(), expected, "{length}");
            assert_eq!(hint.clone().suffix.as_str(), Some(suffix.as_str()));
        }
        assert_eq!(hints("folder").suffix.as_str(), None);
        assert_eq!(hints(".hidden").suffix.as_str(), None);
        assert_eq!(ArchiveExtensionHints::default().retained_bytes(), 0);
    }

    #[test]
    fn stored_suffixes_equal_unicode_case_folding_inline_and_on_the_heap() {
        for suffix in [
            "TXT",
            "Tar.GZ",
            "ABCDEFGHIJKLMN",
            "ABCDEFGHIJKLMNO",
            "photo.2024.JPG",
            "Long.Multi.Part.Suffix.TXT",
            "\u{212A}",
            "STRA\u{1E9E}E",
            "Žluťoučký.ZIP",
            "文档.🗃",
            "e\u{301}",
            "c++",
        ] {
            let folded = UniCase::new(suffix).to_folded_case();
            assert_eq!(
                hints(&format!("name.{suffix}")).suffix.as_str(),
                Some(folded.as_str()),
                "{suffix:?}"
            );
        }
    }

    #[test]
    fn debug_output_keeps_the_former_optional_text_form() {
        assert_eq!(
            format!("{:?}", hints("a.TAR.gz")),
            r#"ArchiveExtensionHints { suffix: Some("tar.gz") }"#
        );
        assert_eq!(
            format!("{:?}", ArchiveExtensionHints::default()),
            "ArchiveExtensionHints { suffix: None }"
        );
    }
}
