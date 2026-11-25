use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// The changed portion on each side of a replacement, in displayed UTF-8 bytes.
/// Callers leave out the line-number gutter and include visible line-ending markers.
pub(super) fn changed_ranges(left: &str, right: &str) -> [Range<usize>; 2] {
    let prefix: usize = left
        .graphemes(true)
        .zip(right.graphemes(true))
        .take_while(|(a, b)| a == b)
        .map(|(cluster, _)| cluster.len())
        .sum();
    let tails = [left, right].map(|line| line[prefix..].graphemes(true));
    let [left_len, right_len] = tails.clone().map(Iterator::count);
    let [left_tail, right_tail] = tails;
    // Align the tails by grapheme count and keep the last run of equal clusters.
    // Walking forward bounds the work even on long runs of regional indicators,
    // whose grapheme boundaries depend on the parity of the preceding run.
    let suffix = left_tail
        .skip(left_len.saturating_sub(right_len))
        .zip(right_tail.skip(right_len.saturating_sub(left_len)))
        .fold(0, |bytes, (a, b)| if a == b { bytes + a.len() } else { 0 });
    [left, right].map(|line| prefix..line.len() - suffix)
}

#[cfg(test)]
mod tests {
    use super::changed_ranges;

    #[test]
    fn inline_changes_keep_shared_text_and_whole_graphemes() {
        for (left, right, expected) in [
            ("name: 12 units", "name: 15 units", ["2", "5"]),
            ("x + y", "x + new y", ["", "new "]),
            ("abc", "ac", ["b", ""]),
            ("wide 文本。", "wide 文件。", ["本", "件"]),
            ("e\u{301} /", "e\u{302} /", ["e\u{301}", "e\u{302}"]),
            ("👩‍💻 /", "👨‍💻 /", ["👩‍💻", "👨‍💻"]),
            ("🇫🇮x", "🇫🇷x", ["🇫🇮", "🇫🇷"]),
            ("one", "one  [CRLF]", ["", "  [CRLF]"]),
            ("one  [no newline]", "one", ["  [no newline]", ""]),
            ("identical", "identical", ["", ""]),
        ] {
            let [a, b] = changed_ranges(left, right);
            assert_eq!([&left[a], &right[b]], expected);
        }
    }

    #[test]
    fn inline_changes_paint_only_paired_lines_and_give_find_priority() {
        use crate::{
            FindQuery, Palette,
            text::{TextComparison, changed_background, tinted},
        };
        use midnight_core::ViewerGeometry;
        use std::sync::atomic::AtomicBool;

        let cancel = AtomicBool::new(false);
        let appearance = crate::text::tests::appearance(14.0);
        let palette = Palette::from(&appearance);
        let paint = |comparison: &mut TextComparison| {
            comparison
                .paint(
                    ViewerGeometry {
                        width: 720,
                        height: 120,
                        scale: 1.0,
                    },
                    &appearance,
                    palette,
                    &cancel,
                )
                .unwrap()
                .view_pixels()
                .2
        };
        let mut pair =
            TextComparison::new("version: old value\n", "version: new value\n", &cancel).unwrap();
        let colors = [0, 1].map(|side| tinted(palette.background, side, 3, 5));
        let both = paint(&mut pair);
        assert!(colors.iter().all(|color| both.contains(color)));

        pair.begin_find(
            FindQuery {
                text: "old".into(),
                ..FindQuery::default()
            },
            false,
            1,
        );
        while pair.find_pending() {
            pair.find_turn(&cancel).unwrap();
        }
        let found = paint(&mut pair);
        assert!(found.contains(&palette.selection));
        assert!(!found.contains(&colors[0]));
        assert!(found.contains(&colors[1]));

        let mut added = TextComparison::new("", "added line\n", &cancel).unwrap();
        let pixels = paint(&mut added);
        assert!(pixels.contains(&changed_background(palette.background, 1)));
        assert!(colors.iter().all(|color| !pixels.contains(color)));
    }
}
