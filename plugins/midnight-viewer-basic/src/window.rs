use midnight_plugin_sdk::api::{
    messages::Viewport,
    viewer::{Anchor, SourceRange, TextPage},
};

// Only decoder-supplied boundaries are eligible: never interpolate UTF-16 or escapes.
pub fn select(mut page: TextPage, bom: u64, anchor: u64, viewport: &Viewport) -> TextPage {
    let last = page.anchors.len() - 1;
    let floor = |offset: u64| {
        page.anchors
            .partition_point(|a| a.source_offset <= offset)
            .saturating_sub(1)
    };
    let (mut start, mut end) = if viewport.backwards {
        (0, floor(anchor))
    } else {
        (floor(anchor), last)
    };
    if viewport.backwards {
        let end_text = page.anchors[end].text_offset as usize;
        let mut lines = 0;
        for i in (start..end).rev() {
            let a = page.anchors[i].text_offset as usize;
            let b = page.anchors[i + 1].text_offset as usize;
            if end_text - a > viewport.max_bytes as usize {
                start = i + 1;
                break;
            }
            if &page.text[a..b] == "\n" && b < end_text {
                lines += 1;
                if lines >= viewport.max_lines {
                    start = i + 1;
                    break;
                }
            }
        }
    } else {
        let first_text = page.anchors[start].text_offset as usize;
        let mut lines = 0;
        for i in start..end {
            let a = page.anchors[i].text_offset as usize;
            let b = page.anchors[i + 1].text_offset as usize;
            if b - first_text > viewport.max_bytes as usize {
                end = i;
                break;
            }
            if &page.text[a..b] == "\n" {
                lines += 1;
                if lines >= viewport.max_lines {
                    end = i + 1;
                    break;
                }
            }
        }
    }
    let first = page.anchors[start].clone();
    let final_anchor = page.anchors[end].clone();
    let starts_line = first.source_offset == bom
        || (first.text_offset > 0 && page.text[..first.text_offset as usize].ends_with('\n'));
    page.has_after |= end < last;
    page.has_before = first.source_offset > bom;
    page.text = page.text[first.text_offset as usize..final_anchor.text_offset as usize].into();
    page.anchors = page.anchors[start..=end]
        .iter()
        .map(|a| Anchor {
            text_offset: a.text_offset - first.text_offset,
            source_offset: a.source_offset,
        })
        .collect();
    page.source = SourceRange {
        offset: first.source_offset,
        length: final_anchor.source_offset - first.source_offset,
    };
    page.partial_line = !starts_line || (page.has_after && !page.text.ends_with('\n'));
    page
}
