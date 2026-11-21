//! `Page::text_range` over a joined page whose every span carries style flags.
use super::*;

/// Eight joined packets of 8 KiB, each with the wire's most spans: 32,768 spans of two
/// bytes, every role, and every combination of the bold, italic and underline flags.
const SPANS: usize = 8 * viewer::MAX_SPANS;

fn flagged_page() -> Page {
    let text: String = (0..SPANS * 2 / 64)
        .map(|_| format!("{}\n", "x".repeat(63)))
        .collect();
    let offset = 1 << 33;
    let page = viewer::TextPage {
        revision: 1,
        source: viewer::SourceRange {
            offset,
            length: text.len() as u64,
        },
        spans: (0..SPANS)
            .map(|index| viewer::Span {
                start: 2 * index as u32,
                end: 2 * index as u32 + 2,
                role: (index % 12) as u16,
                flags: (index % 8) as u8,
            })
            .collect(),
        anchors: (0..=text.len())
            .map(|at| viewer::Anchor {
                text_offset: at as u32,
                source_offset: offset + at as u64,
            })
            .collect(),
        text,
        has_before: true,
        has_after: true,
        partial_line: false,
    };
    let appearance = ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [220; 4],
        background: [20; 4],
        selection: [80; 4],
        accent: [180; 4],
    };
    Page::text(page, &appearance)
}

/// A refill joins the rows past the shown page to a range of it (`Worker::overlap`). That
/// range keeps every span's role and flags, clipped to it, from a page with as many flagged
/// spans as eight joined packets hold, without comparing every span with every style.
#[test]
fn a_range_of_a_page_keeps_the_flags_of_many_styled_spans() {
    let page = flagged_page();
    // Both ends cut a span.
    let range = 3..SPANS * 2 - 3;
    let started = std::time::Instant::now();
    let part = page.text_range(range.clone()).unwrap();
    let elapsed = started.elapsed();
    let expected: Vec<_> = (0..SPANS)
        .filter(|index| 2 * index < range.end && 2 * index + 2 > range.start)
        .map(|index| viewer::Span {
            start: ((2 * index).max(range.start) - range.start) as u32,
            end: ((2 * index + 2).min(range.end) - range.start) as u32,
            role: (index % 12) as u16,
            flags: (index % 8) as u8,
        })
        .collect();
    // The first and last spans lie outside the range.
    assert_eq!(part.spans.len(), SPANS - 2);
    assert!(part.spans == expected, "the spans of the range differ");
    assert_eq!(part.text, page.text[range.clone()]);
    assert_eq!(part.source.offset, page.start + range.start as u64);
    eprintln!("text_range over {SPANS} flagged spans: {elapsed:?}");
}
