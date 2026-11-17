use midnight_plugin_sdk::api::{
    messages::Viewport,
    viewer::{Anchor, SourceRange, Span, TextPage},
};

// Only decoder-supplied boundaries are eligible: never interpolate UTF-16 or escapes.
// A backward page starts no earlier than `limit`: the read's alignment margin before
// it only resynchronizes the decoder and may hold escapes of a split scalar.
pub fn select(
    mut page: TextPage,
    bom: u64,
    anchor: u64,
    limit: u64,
    viewport: &Viewport,
) -> TextPage {
    let last = page.anchors.len() - 1;
    let floor = |offset: u64| {
        page.anchors
            .partition_point(|a| a.source_offset <= offset)
            .saturating_sub(1)
    };
    let (mut start, mut end) = if viewport.backwards {
        (floor(limit), floor(anchor))
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
    let (from, to) = (first.text_offset, final_anchor.text_offset);
    page.has_after |= end < last;
    page.has_before = first.source_offset > bom;
    page.text = page.text[from as usize..to as usize].into();
    // Spans cover whole units, so clipping them at unit boundaries keeps them exact.
    page.spans = page
        .spans
        .iter()
        .filter_map(|span| {
            let (start, end) = (span.start.max(from), span.end.min(to));
            (start < end).then(|| Span {
                start: start - from,
                end: end - from,
                ..span.clone()
            })
        })
        .collect();
    page.anchors = page.anchors[start..=end]
        .iter()
        .map(|a| Anchor {
            text_offset: a.text_offset - from,
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
