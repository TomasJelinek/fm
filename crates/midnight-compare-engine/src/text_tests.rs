use super::*;
use crate::{FontChoices, typeface::resolve_family};
use midnight_plugin_host::{ContentSource, Error};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn installed_choice_wins_and_missing_family_falls_back_to_consolas_then_monospace() {
    let names = ["Cascadia Mono".into(), "Consolas".into()];
    assert_eq!(resolve_family(&names, "Cascadia Mono"), "Cascadia Mono");
    assert_eq!(resolve_family(&names, "consolas"), "Consolas");
    assert_eq!(resolve_family(&names, "Missing Mono"), "Consolas");
    assert_eq!(resolve_family(&names, "monospace"), "monospace");
    assert_eq!(resolve_family(&[], "Consolas"), "monospace");
}

#[test]
fn changing_font_preserves_diff_and_rejects_proportional_or_unknown_families() {
    let mut diff = TextComparison::new("a\nb\n", "A\nB\n", &AtomicBool::new(false)).unwrap();
    diff.typeface.fonts = Some(FontChoices {
        families: vec!["Consolas".into(), "Cascadia Mono".into()].into(),
        selected: "Consolas".into(),
    });
    diff.top = 1;
    diff.horizontal = 24.0;
    assert!(!diff.typeface.change_font("Arial"));
    assert!(!diff.typeface.change_font("Missing Mono"));
    assert!(diff.typeface.change_font("Cascadia Mono"));
    assert_eq!(
        diff.typeface.fonts.as_ref().unwrap().selected,
        "Cascadia Mono"
    );
    assert_eq!(diff.top, 1);
    assert_eq!(diff.horizontal, 24.0);
    assert_eq!(diff.rows.len(), 2);
    assert!(diff.typeface.change_font("monospace"));
}
#[test]
fn aligns_insertions_and_replacements_without_losing_unicode() {
    let cancel = AtomicBool::new(false);
    let diff = TextComparison::new(
        "hello\nčeský 🐈\nlast\n",
        "hello\n世界\nčeský 🐈\nLAST\n",
        &cancel,
    )
    .unwrap();
    assert_eq!(diff.rows.iter().filter(|row| row.changed).count(), 2);
    assert!(
        diff.rows
            .iter()
            .any(|row| row.left() == Some(1) && row.right() == Some(2) && !row.changed)
    );
    assert_eq!(diff.differences.len(), 2);
    assert!(!diff.approximate);
}
#[test]
fn empty_and_missing_newline_are_distinct() {
    let cancel = AtomicBool::new(false);
    assert!(
        TextComparison::new("", "", &cancel)
            .unwrap()
            .rows
            .is_empty()
    );
    assert_eq!(
        TextComparison::new("a\n", "a", &cancel)
            .unwrap()
            .differences
            .len(),
        1
    );
}
#[test]
fn form_feed_vertical_tab_and_ansi_escape_are_text_shown_as_control_pictures() {
    let text = "page 1\u{0c}\n\u{1b}[31mred\u{1b}[0m\u{0b}x\n";
    assert_eq!(decode(text.as_bytes()).unwrap(), text);
    for binary in ["a\u{1}b", "a\u{7f}b", "a\u{85}b"] {
        assert!(decode(binary.as_bytes()).is_err(), "{binary:?}");
    }
    assert_eq!(visible_controls("a\u{0c}b\u{1b}c\u{0b}"), "a␌b␛c␋");
    assert_eq!(visible_controls("tab\tkept"), "tab\tkept");
    let mut diff = TextComparison::new(text, "page 1\n", &AtomicBool::new(false)).unwrap();
    let geometry = ViewerGeometry {
        width: 400,
        height: 100,
        scale: 1.0,
    };
    let frame = diff
        .paint(
            geometry,
            &appearance(12.0),
            Palette::from(&appearance(12.0)),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(frame.accessible_text.contains("page 1␌"));
    assert!(frame.accessible_text.contains("␛[31mred␛[0m␋x"));
    assert!(!frame.bytes && !frame.bytes_only);
}

#[test]
fn bidi_controls_are_shown_as_escapes() {
    assert_eq!(
        visible_controls("a\u{202e}b\u{2066}c\u{200f}d\u{1b}"),
        "a\\u202Eb\\u2066c\u{200f}d␛"
    );
    for control in ('\u{202a}'..='\u{202e}').chain('\u{2066}'..='\u{2069}') {
        let line = format!("x{control}y");
        assert_eq!(
            visible_controls(&line),
            format!("x\\u{:04X}y", u32::from(control))
        );
    }
    assert!(matches!(
        visible_controls("RTL \u{200e}\u{200f}\u{61c} marks"),
        std::borrow::Cow::Borrowed(_)
    ));
}

#[test]
fn lines_that_differ_only_in_a_bidi_control_show_both_escapes_in_one_changed_row() {
    let cancel = AtomicBool::new(false);
    let mut diff = TextComparison::new("x\u{202e}y\n", "x\u{202c}y\n", &cancel).unwrap();
    assert_eq!(diff.rows.len(), 1);
    assert!(diff.rows[0].changed);
    let geometry = ViewerGeometry {
        width: 400,
        height: 100,
        scale: 1.0,
    };
    let frame = diff
        .paint(
            geometry,
            &appearance(12.0),
            Palette::from(&appearance(12.0)),
            &cancel,
        )
        .unwrap();
    assert!(frame.accessible_text.contains("x\\u202Ey"));
    assert!(frame.accessible_text.contains("x\\u202Cy"));
    assert!(
        !frame
            .accessible_text
            .contains(midnight_core::is_explicit_bidi_control)
    );
}

pub(crate) fn appearance(font_size: f32) -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size,
        foreground: [255; 4],
        background: [0, 0, 0, 255],
        selection: [40, 60, 90, 255],
        accent: [90, 150, 220, 255],
    }
}

#[test]
fn pages_advance_by_fully_visible_rows_at_every_font_size() {
    let left: String = (0..400).map(|row| format!("row {row}\n")).collect();
    let cancel = AtomicBool::new(false);
    for (font_size, scale) in [(12.0, 1.0), (15.0, 1.0), (20.0, 1.0), (12.0, 2.0)] {
        let mut diff = TextComparison::new(&left, &left, &cancel).unwrap();
        let height = 300;
        let geometry = ViewerGeometry {
            width: 400,
            height,
            scale,
        };
        diff.paint(
            geometry,
            &appearance(font_size),
            Palette::from(&appearance(font_size)),
            &cancel,
        )
        .unwrap();
        let row_height = (font_size * scale * 1.6).ceil();
        let visible = (height as f32 / row_height).floor() as usize;
        diff.scroll_pages(1);
        assert_eq!(diff.top, visible, "font {font_size} at scale {scale}");
        diff.scroll_pages(-1);
        assert_eq!(diff.top, 0);
        // Wheel pixels are logical; one painted row height scrolls exactly one row.
        diff.scroll_pixels(-(row_height / scale));
        assert_eq!(diff.top, 1, "font {font_size} at scale {scale}");
    }
}

#[test]
fn rejects_binary_invalid_encoding_and_limits() {
    assert!(decode(b"a\0b").is_err());
    assert!(decode(&[0xff]).is_err());
    assert!(decode(&[0xff, 0xfe, 1]).is_err());
    assert!(decode(&vec![b'a'; MAX_LINE_BYTES + 1]).is_err());
    assert_eq!(decode(&[0xff, 0xfe, 0xe9, 0]).unwrap(), "é");
    assert_eq!(decode(b"\xef\xbb\xbfbom").unwrap(), "bom");
    let lines = "\n".repeat(MAX_LINES);
    assert!(decode(lines.as_bytes()).is_ok());
    let error = decode(format!("{lines}\n").as_bytes()).unwrap_err();
    assert!(error.contains("1,000,000 lines"), "{error}");
}
#[test]
fn cancellation_and_navigation() {
    assert!(TextComparison::new("a", "b", &AtomicBool::new(true)).is_err());
    let mut diff = TextComparison::new("a\nb\nc\n", "A\nb\nC\n", &AtomicBool::new(false)).unwrap();
    diff.difference(false);
    assert_eq!(diff.top, 2);
    diff.difference(true);
    assert_eq!(diff.top, 0);
    diff.seek(f64::NAN);
    assert_eq!(diff.top, 0);
}
#[test]
fn difference_navigation_stops_at_each_boundary_and_refocuses_after_scrolling() {
    let mut diff = TextComparison::new(
        "a\nb\nc\nd\ne\n",
        "A\nb\nC\nd\nE\n",
        &AtomicBool::new(false),
    )
    .unwrap();
    diff.difference(true);
    assert_eq!(diff.top, 0, "Previous must not wrap to the last group");
    diff.difference(false);
    assert_eq!(diff.top, 2);
    diff.difference(false);
    assert_eq!(diff.top, 4);
    diff.difference_edge(false);
    assert_eq!(diff.top, 0);
    diff.difference_edge(true);
    assert_eq!(diff.top, 4);
    diff.difference(false);
    assert_eq!(diff.top, 4, "Next must not wrap to the first group");
    diff.scroll(-1);
    diff.difference(false);
    assert_eq!(diff.top, 4);
    let mut empty = TextComparison::new("", "", &AtomicBool::new(false)).unwrap();
    empty.difference(true);
    empty.difference(false);
    empty.difference_edge(false);
    empty.difference_edge(true);
    assert_eq!(empty.top, 0);
}
#[test]
fn captured_source_change_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("český.txt");
    std::fs::write(&path, "before").unwrap();
    let file = CapturedFile::capture(&path).unwrap();
    std::fs::write(&path, "after mutation").unwrap();
    assert!(matches!(
        read(&file, false, &AtomicBool::new(false), &mut |_| {}),
        Err(ReadError::Failed(_))
    ));
}

/// Generated content of a given length, counting its reads.
#[derive(Debug)]
struct Generated {
    length: u64,
    byte: u8,
    reads: AtomicUsize,
}

impl ContentSource for Generated {
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        "generated.log".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        let end = self.length.min(offset + u64::from(length));
        Ok((offset..end)
            .map(|index| if index % 64 == 63 { b'\n' } else { self.byte })
            .collect())
    }
}

fn generated(length: u64, byte: u8) -> (Arc<Generated>, CapturedFile) {
    let source = Arc::new(Generated {
        length,
        byte,
        reads: AtomicUsize::new(0),
    });
    let file = CapturedFile::from_source(source.clone()).unwrap();
    (source, file)
}

#[test]
fn oversized_and_binary_files_are_refused_as_text_without_reading_them_whole() {
    let cancel = AtomicBool::new(false);
    let (source, file) = generated(MAX_BYTES + 1, b'a');
    let Err(ReadError::Refused(reason)) = read(&file, false, &cancel, &mut |_| {}) else {
        panic!("a file over the limit must be refused as text");
    };
    assert!(reason.contains("32 MiB"), "{reason}");
    assert_eq!(source.reads.load(Ordering::Acquire), 0);

    let (source, file) = generated(4 * 1024 * 1024, 0);
    let Err(ReadError::Refused(reason)) = read(&file, false, &cancel, &mut |_| {}) else {
        panic!("NUL bytes are not text");
    };
    assert!(reason.contains("not UTF-8 or UTF-16 text"), "{reason}");
    assert_eq!(
        source.reads.load(Ordering::Acquire),
        1,
        "the first block decides"
    );
}

#[test]
fn large_text_reports_reading_progress_and_is_compared() {
    let cancel = AtomicBool::new(false);
    let length = 9 * 1024 * 1024;
    let (_, left) = generated(length, b'a');
    let (_, right) = generated(length, b'b');
    let mut reported = Vec::new();
    let left = read(&left, false, &cancel, &mut |bytes| reported.push(bytes)).unwrap();
    assert_eq!(reported, [4 << 20, 8 << 20]);
    let right = read(&right, false, &cancel, &mut |_| {}).unwrap();
    assert_eq!(left.lines.len(), length as usize / 64);
    let diff = TextComparison::from_lines([left.lines, right.lines], &cancel).unwrap();
    assert_eq!(diff.rows.len(), length as usize / 64);
    assert!(diff.rows.iter().all(|row| row.changed));
    assert_eq!(diff.differences, [0]);
    assert!(
        diff.approximate,
        "nothing anchors two unrelated texts, so they are one changed block"
    );
}

#[test]
fn cancellation_stops_reading() {
    let (_, file) = generated(1024 * 1024, b'a');
    let cancel = AtomicBool::new(false);
    let mut calls = 0;
    // Reading stops at the next block after the request.
    let mut progress = |_: u64| calls += 1;
    cancel.store(true, Ordering::Release);
    assert!(matches!(
        read(&file, false, &cancel, &mut progress),
        Err(ReadError::Failed(_))
    ));
    assert_eq!(calls, 0);
}

#[test]
fn confirmed_ssd_reads_share_the_io_gate_and_unknown_storage_stays_exclusive() {
    #[derive(Debug)]
    struct Observed(Mutex<Vec<midnight_work_budget::Snapshot>>);
    impl ContentSource for Observed {
        fn length(&self) -> u64 {
            2
        }
        fn display_name(&self) -> String {
            "observed.txt".into()
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
            self.0
                .lock()
                .unwrap()
                .push(midnight_work_budget::shared_io().snapshot());
            Ok(b"a\n".to_vec())
        }
    }
    for confirmed_ssd in [true, false] {
        let source = std::sync::Arc::new(Observed(Default::default()));
        let file = CapturedFile::from_source(source.clone()).unwrap();
        assert_eq!(
            read(&file, confirmed_ssd, &AtomicBool::new(false), &mut |_| {})
                .unwrap()
                .lines
                .text(),
            "a\n"
        );
        let snapshot = source.0.lock().unwrap()[0];
        if confirmed_ssd {
            assert!(!snapshot.exclusive && snapshot.active_shared >= 1);
        } else {
            assert!(snapshot.exclusive);
        }
    }
}

#[test]
fn the_byte_check_of_control_characters_matches_char_is_control() {
    let samples = [
        "plain ascii text\n",
        "tab\tand CRLF\r\n",
        "\u{1b}[31m\u{0b}\u{0c}",
        "a\u{85}b",
        "ü\u{80}",
        "\u{9f}",
        "x\u{7f}",
        "\u{0}",
        "\u{1f}",
        "Â¢\u{a0}NBSP",
        "€ 中文 🐈",
        "\u{c2}\u{c2}",
    ];
    for sample in samples {
        // Every position of the sample within an eight-byte word.
        for shift in 0..9 {
            for suffix in ["", "z", "zzzzzzzzz"] {
                let text = format!("{}{sample}{suffix}", "a".repeat(shift));
                let expected = text.chars().any(|c| c.is_control() && !text_control(c));
                assert_eq!(has_binary_control(&text), expected, "{text:?}");
            }
        }
    }
}

#[test]
fn thousands_are_grouped() {
    assert_eq!(grouped(0), "0");
    assert_eq!(grouped(999), "999");
    assert_eq!(grouped(1000), "1,000");
    assert_eq!(grouped(8192), "8,192");
    assert_eq!(grouped(1_000_000), "1,000,000");
    assert_eq!(grouped(u64::MAX), "18,446,744,073,709,551,615");
}
