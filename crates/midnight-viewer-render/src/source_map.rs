//! Layout-only tab/newline expansion. Clipboard text always uses the original.
use crate::{LayoutError, MAX_TEXT_BYTES};
use std::{borrow::Cow, ops::Range};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

struct Replacement {
    source: Range<usize>,
    shaped: Range<usize>,
}

#[derive(Default)]
pub(crate) struct SourceMap(Vec<Replacement>);

impl SourceMap {
    pub(crate) fn prepare(text: &str, tab_width: u8) -> Result<(Cow<'_, str>, Self), LayoutError> {
        if !text.contains(['\r', '\t']) {
            return Ok((Cow::Borrowed(text), Self::default()));
        }
        let mut shaped = String::with_capacity(text.len());
        let mut replacements = Vec::new();
        let mut column = 0;
        for (index, grapheme) in text.grapheme_indices(true) {
            let start = shaped.len();
            let changed = match grapheme {
                "\r" | "\r\n" => {
                    shaped.push('\n');
                    column = 0;
                    true
                }
                "\t" => {
                    let count = usize::from(tab_width) - column % usize::from(tab_width);
                    if shaped.len() + count > MAX_TEXT_BYTES {
                        return Err(LayoutError::LimitExceeded);
                    }
                    shaped.extend(std::iter::repeat_n(' ', count));
                    column += count;
                    true
                }
                _ => {
                    shaped.push_str(grapheme);
                    if grapheme.chars().any(crate::is_line_break) {
                        column = 0;
                    } else {
                        column += grapheme.width();
                    }
                    false
                }
            };
            if shaped.len() > MAX_TEXT_BYTES {
                return Err(LayoutError::LimitExceeded);
            }
            if changed {
                if replacements.len() == 16384 {
                    return Err(LayoutError::LimitExceeded);
                }
                replacements.push(Replacement {
                    source: index..index + grapheme.len(),
                    shaped: start..shaped.len(),
                });
            }
        }
        Ok((Cow::Owned(shaped), Self(replacements)))
    }

    pub(crate) fn to_layout(&self, source: usize) -> usize {
        let count = self.0.partition_point(|edit| edit.source.start < source);
        let Some(edit) = count.checked_sub(1).map(|i| &self.0[i]) else {
            return source;
        };
        if source < edit.source.end {
            edit.shaped.start
        } else {
            edit.shaped.end + (source - edit.source.end)
        }
    }

    pub(crate) fn to_source(&self, shaped: usize) -> usize {
        let count = self.0.partition_point(|edit| edit.shaped.start < shaped);
        let Some(edit) = count.checked_sub(1).map(|i| &self.0[i]) else {
            return shaped;
        };
        if shaped < edit.shaped.end {
            // A tab is one source character: select the closest edge, never
            // expose invented spaces or an interior CRLF boundary to the user.
            if shaped - edit.shaped.start < edit.shaped.end - shaped {
                edit.source.start
            } else {
                edit.source.end
            }
        } else {
            edit.source.end + (shaped - edit.shaped.end)
        }
    }
}
