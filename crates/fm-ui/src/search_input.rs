//! Text editing concerns only the small query, never directory names or matching.
use unicode_segmentation::UnicodeSegmentation;

fn byte_offset(text: &str, offset: i32) -> usize {
    let mut offset = usize::try_from(offset).unwrap_or(0).min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

pub(crate) fn erase_previous_grapheme(text: &str, anchor: i32, cursor: i32) -> (String, usize) {
    let anchor = byte_offset(text, anchor);
    let cursor = byte_offset(text, cursor);
    let (start, end) = if anchor != cursor {
        let start = anchor.min(cursor);
        let end = anchor.max(cursor);
        let mut bounds = (start, end);
        for (offset, cluster) in text.grapheme_indices(true) {
            let after = offset + cluster.len();
            if offset < start && start < after {
                bounds.0 = offset;
            }
            if offset < end && end < after {
                bounds.1 = after;
                break;
            }
            if offset >= end {
                break;
            }
        }
        bounds
    } else if cursor == 0 {
        return (text.to_owned(), 0);
    } else {
        text.grapheme_indices(true)
            .rev()
            .find(|(offset, _)| *offset < cursor)
            .map_or((0, cursor), |(offset, cluster)| {
                (offset, offset + cluster.len())
            })
    };
    let mut edited = String::with_capacity(text.len() - (end - start));
    edited.push_str(&text[..start]);
    edited.push_str(&text[end..]);
    (edited, start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backspace_removes_complete_combining_emoji_flag_and_indic_clusters() {
        for text in ["e\u{301}", "👩🏽‍💻", "🇨🇿", "क्\u{200d}ष"] {
            let end = text.len() as i32;
            assert_eq!(erase_previous_grapheme(text, end, end), (String::new(), 0));
        }
        let text = "before👩🏽‍💻after";
        let cursor = "before👩🏽‍💻".len() as i32;
        assert_eq!(
            erase_previous_grapheme(text, cursor, cursor),
            ("beforeafter".into(), 6)
        );
    }

    #[test]
    fn query_selection_and_stale_offsets_cannot_split_utf8_or_panic() {
        assert_eq!(erase_previous_grapheme("a文档z", 1, 7), ("az".into(), 1));
        assert_eq!(erase_previous_grapheme("a文档z", 7, 1), ("az".into(), 1));
        assert_eq!(erase_previous_grapheme("文", 2, 2), ("文".into(), 0));
        assert_eq!(
            erase_previous_grapheme("文", i32::MAX, i32::MAX),
            (String::new(), 0)
        );
        assert_eq!(erase_previous_grapheme("文", -1, -1), ("文".into(), 0));
    }
}
