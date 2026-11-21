//! Shaping runs short enough for parley's cluster offsets.
//!
//! Parley 0.11 keeps a glyph cluster's byte offset inside its shaping run in 16
//! bits (`ClusterData::text_offset`), and uniformly styled text is one run, so
//! past 64 KiB clusters named the wrong text: carets, hit tests, selections and
//! line ranges went wrong. Parley starts a new run where the font features
//! change. A shaper drops a feature whose value is zero before it looks the
//! feature up in the font (harfrust's `ot_map`, as in HarfBuzz), so giving every
//! other piece of a longer layout an extra disabled feature bounds each run
//! without changing a glyph or an advance. Pieces end at line starts where they
//! can: shaping never joins glyphs across a line break.
use crate::{LayoutError, is_line_break};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// The longest run whose cluster offsets fit in 16 bits.
pub(crate) const RUN_BYTES: usize = 1 << 16;

/// The font features of every other piece: the page's own and a disabled tag
/// that no shaper applies.
pub(crate) fn alternate_features(tabular_numbers: bool) -> &'static str {
    if tabular_numbers {
        "\"tnum\", \"MRUN\" off"
    } else {
        "\"MRUN\" off"
    }
}

/// The second, fourth and further pieces of `text` cut into pieces of at most
/// `limit` bytes (see [`piece_starts`]).
pub(crate) fn alternate_pieces(text: &str, limit: usize) -> Result<Vec<Range<usize>>, LayoutError> {
    Ok(piece_starts(text, limit)?
        .chunks(2)
        .map(|pair| pair[0]..pair.get(1).copied().unwrap_or(text.len()))
        .collect())
}

/// Where the pieces after the first start when `text`, laid-out text whose line
/// ends are LF (`SourceMap`), is cut into pieces of at most `limit` bytes. A
/// piece ends at the last line start that fits. A line longer than `limit` is cut
/// after its last whitespace that fits, else at its last grapheme boundary that
/// does. Only one grapheme cluster longer than `limit` is cut between two of its
/// characters, which keeps carets right: parley counts a cluster's glyphs in 8
/// bits (`ClusterData::glyph_len`), so it draws such a cluster wrong anyway. A
/// `limit` shorter than a character is refused.
fn piece_starts(text: &str, limit: usize) -> Result<Vec<usize>, LayoutError> {
    let mut starts = Vec::new();
    let mut start = 0;
    while text.len() - start > limit {
        start += piece_length(&text[start..], limit).ok_or(LayoutError::LimitExceeded)?;
        starts.push(start);
    }
    Ok(starts)
}

/// The length of the first piece of `text`, which is longer than `limit` bytes.
fn piece_length(text: &str, limit: usize) -> Option<usize> {
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(index) = text[..end].rfind(is_line_break) {
        return Some(index + text[index..].chars().next()?.len_utf8());
    }
    let mut after_space = None;
    let mut boundary = None;
    let mut space = false;
    for (index, grapheme) in text.grapheme_indices(true) {
        if index > limit {
            break;
        }
        if index > 0 {
            boundary = Some(index);
            if space {
                after_space = Some(index);
            }
        }
        space = grapheme.chars().all(char::is_whitespace);
    }
    after_space.or(boundary).or((end > 0).then_some(end))
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
