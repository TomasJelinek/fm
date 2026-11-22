//! Source highlighting while scrolling: the frontier co-simulation (`worker_frontier_tests.rs`)
//! with a model of the Source viewer's lexical state (`Lexer`), over synthetic C++ of the
//! shape of a user's file (150 rows, about 6 KB, rows of up to about 120 bytes) and of
//! 2 MiB. It counts the preparations the worker asks for, the refreshes and the notices the
//! status line shows, per gesture.
use super::frontier::{Engine, NOTCH, Window, rest, try_gesture};
use super::*;
use std::collections::{BTreeSet, VecDeque};
use std::ops::Range;

/// A checkpoint at most this far before a read's anchor lets the Source viewer highlight
/// it (`plugins/midnight-viewer-source/src/paging.rs`, `plan`), and at most `TARGET_REACH`
/// before its target.
const REACH: u64 = 32 * 1024;
const TARGET_REACH: u64 = 65_528;
/// Checkpoints the Source viewer keeps spread over the document, and of recent lines; the
/// spread ones start this far apart and double their spacing when full (`checkpoints.rs`).
const SPREAD: usize = 128;
const RECENT: usize = 126;
const INITIAL_SPACING: u64 = 2048;
/// A lexed stretch keeps 17 line ends at least 4 KiB apart and its last 64 (`Batch`).
const BATCH_SPREAD: usize = 17;
const BATCH_SPACING: u64 = 4096;
const BATCH_RECENT: usize = 64;
/// A preparation reads 16 KiB at a time, at most 64 times per request (`preparation.rs`).
const READ_BYTES: u64 = 16 * 1024;
const MAX_REPLIES: u64 = 64;
/// Lookahead lexed past a page's or a prepared target's visible end.
const LOOKAHEAD: u64 = 128;

/// The Source viewer's lexical state as the co-simulation models it: line ends whose lexer
/// state it keeps (`plugins/midnight-viewer-source/src/checkpoints.rs`), a highlighted read
/// for each page with one close enough before it (`paging.rs`), and a preparation that walks
/// from the checkpoint nearest before its target, keeping the line ends it passes, until the
/// target and its lookahead are lexed (`preparation.rs`). Only the reserved target of a ready
/// preparation is read highlighted without a checkpoint near it, once.
pub(super) struct Lexer {
    spread: BTreeSet<u64>,
    spacing: u64,
    recent: VecDeque<u64>,
    /// The viewport a preparation walks towards, whether it is ready, and where it stands.
    target: Option<messages::Viewport>,
    ready: bool,
    cursor: Option<u64>,
    /// Bytes the preparations walked, and pages read highlighted and plain.
    pub walked: u64,
    pub highlighted: usize,
    pub plain: usize,
}

impl Lexer {
    pub fn new() -> Self {
        let mut lexer = Self {
            spread: BTreeSet::new(),
            spacing: INITIAL_SPACING,
            recent: VecDeque::new(),
            target: None,
            ready: false,
            cursor: None,
            walked: 0,
            highlighted: 0,
            plain: 0,
        };
        // The state at the document's start (after its byte order mark) is always known.
        lexer.insert(0);
        lexer
    }

    fn crowded(&self, offset: u64) -> bool {
        let before = self.spread.range(..offset).next_back();
        let after = self.spread.range(offset..).next();
        before.is_some_and(|at| offset - at < self.spacing)
            || after.is_some_and(|at| at - offset < self.spacing)
    }

    fn insert(&mut self, offset: u64) {
        while !self.spread.contains(&offset) && !self.crowded(offset) {
            if self.spread.len() < SPREAD {
                self.spread.insert(offset);
                break;
            }
            self.spacing *= 2;
            let spacing = self.spacing;
            let mut kept: Option<u64> = None;
            self.spread.retain(|&at| {
                let keep = kept.is_none_or(|previous| at - previous >= spacing);
                if keep {
                    kept = Some(at);
                }
                keep
            });
        }
        if !self.recent.contains(&offset) {
            if self.recent.len() == RECENT {
                self.recent.pop_front();
            }
            self.recent.push_back(offset);
        }
    }

    fn nearest(&self, anchor: u64) -> Option<u64> {
        let spread = self.spread.range(..=anchor).next_back().copied();
        self.recent
            .iter()
            .copied()
            .filter(|at| *at <= anchor)
            .chain(spread)
            .max()
    }

    /// Keep the line ends of the lexed `range` of `source` as one batch does.
    fn commit(&mut self, source: &str, range: Range<u64>) {
        let start = range.start as usize;
        let ends: Vec<u64> = source.as_bytes()[start..range.end as usize]
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(at, _)| (start + at + 1) as u64)
            .collect();
        let mut spread: Vec<u64> = Vec::new();
        for end in &ends {
            if spread.len() < BATCH_SPREAD
                && spread.last().is_none_or(|last| end - last >= BATCH_SPACING)
            {
                spread.push(*end);
            }
        }
        let recent = &ends[ends.len().saturating_sub(BATCH_RECENT)..];
        for at in spread.into_iter().chain(recent.iter().copied()) {
            self.insert(at);
        }
    }

    /// Whether the Source viewer highlights `page`, its reply to `viewport`, keeping the line
    /// ends it lexed.
    pub fn read(
        &mut self,
        source: &str,
        viewport: &messages::Viewport,
        page: &viewer::TextPage,
    ) -> bool {
        let start = if self.ready && self.target.as_ref() == Some(viewport) {
            self.ready = false;
            Some(lower(viewport))
        } else {
            let limit = u64::from(viewport.max_bytes.min(8192));
            let lines = u64::from(viewport.max_lines);
            let upper = viewport.source_anchor;
            let anchor = if viewport.backwards {
                upper.saturating_sub(limit + lines + 1 + 8)
            } else {
                upper
            };
            let target = if viewport.backwards { upper } else { anchor };
            self.nearest(anchor)
                .filter(|at| anchor - at <= REACH && target - at <= TARGET_REACH)
        };
        let Some(start) = start else {
            self.plain += 1;
            return false;
        };
        self.highlighted += 1;
        let end = (page.source.offset + page.source.length + LOOKAHEAD).min(source.len() as u64);
        self.commit(source, start.min(end)..end);
        true
    }

    /// The Source viewer's walk towards `request`'s target: what it answers, and the bytes it
    /// walked for it.
    pub fn prepare(
        &mut self,
        source: &str,
        request: &messages::PrepareSource,
    ) -> (messages::SourcePrepared, u64) {
        let viewport = &request.viewport;
        let lower = lower(viewport);
        let initial = self.nearest(lower).unwrap_or(0);
        let continuing = self.target.as_ref() == Some(viewport);
        if !continuing {
            self.ready = false;
        }
        self.target = Some(viewport.clone());
        let cursor = self
            .cursor
            .filter(|cursor| continuing || (initial <= *cursor && *cursor <= lower))
            .unwrap_or(initial);
        let walked = if self.ready {
            0
        } else {
            let goal = goal(source, viewport);
            let budget = u64::from(request.max_scan_bytes).min(READ_BYTES * MAX_REPLIES);
            let stop = goal.min(cursor + budget).max(cursor);
            let mut at = cursor;
            while at < stop {
                let next = (at + READ_BYTES).min(stop);
                self.commit(source, at..next);
                at = next;
            }
            self.cursor = Some(stop);
            self.ready = stop >= goal;
            stop - cursor
        };
        self.walked += walked;
        (
            messages::SourcePrepared {
                revision: request.revision,
                frontier: self.cursor.unwrap_or(cursor),
                ready: self.ready,
            },
            walked,
        )
    }
}

/// Where a preparation captures its target (`preparation::lower`).
fn lower(viewport: &messages::Viewport) -> u64 {
    if viewport.backwards {
        viewport
            .source_anchor
            .saturating_sub(u64::from(viewport.max_bytes.min(8192)) * 2 + 8)
    } else {
        viewport.source_anchor
    }
}

/// Where a walk towards `viewport` is ready: past its visible rows and their lookahead
/// (`Scan::measure`).
fn goal(source: &str, viewport: &messages::Viewport) -> u64 {
    let anchor = viewport.source_anchor as usize;
    let end = if viewport.backwards {
        anchor
    } else {
        let limit = viewport.max_bytes.min(8192) as usize;
        let mut end = anchor;
        let mut lines = 0;
        for character in source[anchor..].chars() {
            if end - anchor + character.len_utf8() > limit || lines >= viewport.max_lines {
                break;
            }
            end += character.len_utf8();
            lines += u32::from(character == '\n');
        }
        end
    };
    (end as u64 + LOOKAHEAD).min(source.len() as u64)
}

/// The rows of synthetic C++, `{n}` standing for the block's number. 150 of them make 5.9 KB
/// with rows of up to 116 bytes, the shape of the user's file this was measured for (150
/// rows, 6.0 KB, up to 120 bytes, LF line ends, no tabs, `//` comments).
const CPP: [&str; 30] = [
    "// Generated C++ for the viewer scenarios: block {n} of the suite, formatted as a source file.",
    "#include <cstddef>",
    "#include <string>",
    "#include <vector>",
    "",
    "namespace generated_{n} {",
    "",
    "// Joins the values of one record with commas, padded to the column width when the caller asks.",
    "static std::string join_values_{n}(const std::vector<int>& values, std::size_t width, bool padded, char fill)",
    "{",
    "    std::string text;",
    "    text.reserve(values.size() * 8 + 16);",
    "    for (std::size_t index = 0; index < values.size(); ++index) {",
    "        if (index != 0) {",
    "            text += \", \";",
    "        }",
    "        text += std::to_string(values[index] * {n} + 17);",
    "    }",
    "    if (padded && text.size() < width) {",
    "        text.append(width - text.size(), fill);",
    "    }",
    "    return text;",
    "}",
    "",
    "static const char* const kLabels_{n}[] = {\"first\", \"second\", \"third\", \"fourth\", \"fifth\", \"sixth\"};",
    "constexpr int kLimit_{n} = {n} * 64 + 1024; // The most values that one record of this block can hold.",
    "int clamp_count_{n}(int value) { return value > kLimit_{n} ? kLimit_{n} : (value < 0 ? 0 : value); }",
    "std::size_t padded_width_{n}(std::size_t width, std::size_t columns) { return width * columns + (columns + 1) / 2 + 1; }",
    "} // namespace generated_{n}",
    "",
];

/// `rows` rows of synthetic C++ (`CPP`).
pub(super) fn cpp(rows: usize) -> String {
    let mut text = String::new();
    let (mut block, mut row) = (0, 0);
    while row < rows {
        let number = block.to_string();
        for line in CPP.iter().take(rows - row) {
            text.push_str(&line.replace("{n}", &number));
            text.push('\n');
        }
        row += CPP.len().min(rows - row);
        block += 1;
    }
    text
}

const KEYWORDS: [&str; 14] = [
    "bool",
    "char",
    "const",
    "constexpr",
    "for",
    "if",
    "include",
    "int",
    "namespace",
    "return",
    "static",
    "std",
    "size_t",
    "string",
];

/// The tokens the Source viewer colours in `text`: comments, strings, numbers and keywords.
fn tokens(text: &str) -> Vec<(Range<usize>, u16)> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &text[at..];
        let (length, role) = if rest.starts_with("//") {
            (rest.find('\n').unwrap_or(rest.len()), viewer::STYLE_COMMENT)
        } else if let Some(quoted) = rest.strip_prefix('"') {
            (
                quoted.find('"').map_or(rest.len(), |end| end + 2),
                viewer::STYLE_STRING,
            )
        } else if bytes[at].is_ascii_digit() {
            (
                rest.find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len()),
                viewer::STYLE_NUMBER,
            )
        } else if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            let length = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            if !KEYWORDS.contains(&&rest[..length]) {
                at += length;
                continue;
            }
            (length, viewer::STYLE_KEYWORD)
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        tokens.push((at..at + length, role));
        at += length.max(1);
    }
    tokens
}

/// Colour spans as the Source viewer sends them for the synthetic C++ (`tokens`), at most
/// `viewer::MAX_SPANS`.
pub(super) fn highlight_cpp(page: &mut viewer::TextPage) {
    page.spans = tokens(&page.text)
        .into_iter()
        .take(viewer::MAX_SPANS)
        .map(|(range, role)| viewer::Span {
            start: range.start as u32,
            end: range.end as u32,
            role,
            flags: 0,
        })
        .collect();
}

/// Whether the window shows a row with a token that its page has no colour in.
pub(super) fn shows_plain_rows(engine: &Engine) -> bool {
    let worker = &engine.worker;
    let (Some(page), Some(shaped)) = (&worker.page, &worker.shaped) else {
        return false;
    };
    let top = worker.origin[1];
    let bottom = top + worker.geometry.height as f32;
    let (Ok((first, _)), Ok((last, _))) = (shaped.line_at_y(top), shaped.line_at_y(bottom)) else {
        return false;
    };
    let mut at = first.start.min(page.text.len());
    for row in page.text[at..last.end.min(page.text.len()).max(at)].split_inclusive('\n') {
        let range = at..at + row.len();
        at = range.end;
        let coloured = page
            .roles
            .iter()
            .any(|role| role.range.start < range.end && range.start < role.range.end);
        if !coloured && !tokens(row).is_empty() {
            return true;
        }
    }
    false
}

fn geometry() -> ViewerGeometry {
    ViewerGeometry {
        width: 1312,
        height: 720,
        scale: 1.0,
    }
}

/// A Source view of `source` through the lexical model, opened at its start and at rest.
pub(super) fn source_view(source: String) -> (Engine, Window) {
    let mut engine = Engine::open(source, viewer::MODE_SOURCE, geometry(), 1).highlighted();
    engine.lexer = Some(Lexer::new());
    engine.highlight_replies = Some(highlight_cpp);
    let mut window = Window::default();
    rest(&mut engine, &mut window);
    (engine, window)
}

/// What a gesture made the worker and the plugin do.
#[derive(Debug, Default, PartialEq, Eq)]
struct Work {
    preparations: usize,
    refreshes: usize,
    notices: usize,
    /// Frames that showed a row with a token uncoloured.
    plain_frames: usize,
}

/// Run `gesture` from rest to rest, frame by frame, and count its work.
fn measure(engine: &mut Engine, window: &mut Window, gesture: &Gesture) -> Work {
    rest(engine, window);
    let before = (engine.preparations, engine.refreshes, engine.notices);
    let mut plain_frames = 0;
    match gesture {
        Gesture::Inputs(inputs) => {
            for (pixels, direct) in inputs {
                window.push(engine, *pixels, *direct);
                super::frontier::frame(engine, window);
                plain_frames += usize::from(shows_plain_rows(engine));
            }
        }
        Gesture::Drag(positions) => {
            for position in positions {
                window.seek(engine, *position);
                super::frontier::frame(engine, window);
                plain_frames += usize::from(shows_plain_rows(engine));
            }
        }
    }
    for _ in 0..2000 {
        super::frontier::frame(engine, window);
        plain_frames += usize::from(shows_plain_rows(engine));
        if !window.active() && !engine.busy() && !engine.worker.dirty {
            break;
        }
    }
    rest(engine, window);
    Work {
        preparations: engine.preparations - before.0,
        refreshes: engine.refreshes - before.1,
        notices: engine.notices - before.2,
        plain_frames,
    }
}

enum Gesture {
    /// Wheel notches or keys, one a frame, in physical pixels (keys are direct).
    Inputs(Vec<(f64, bool)>),
    /// Scrollbar positions, one a frame.
    Drag(Vec<f64>),
}

/// The gestures of the scrolling measurements (`worker_prepared_tests::scripts`), one input a
/// frame: wheel notches down and up, a touchpad flick's decaying steps, PageDowns and a
/// scrollbar drag from 30 % to 36 %.
fn gestures() -> Vec<(&'static str, Gesture)> {
    let flick = (0..90)
        .map(|frame| {
            let seconds = f64::from(frame) / 60.0;
            (NOTCH * 30.0 * (-seconds / 0.5).exp() / 60.0, false)
        })
        .collect();
    vec![
        (
            "wheel 12 notches down",
            Gesture::Inputs(vec![(NOTCH, false); 12]),
        ),
        (
            "wheel 12 notches up",
            Gesture::Inputs(vec![(-NOTCH, false); 12]),
        ),
        ("touchpad flick", Gesture::Inputs(flick)),
        ("PageDown x8", Gesture::Inputs(vec![(720.0, true); 8])),
        ("PageUp x8", Gesture::Inputs(vec![(-720.0, true); 8])),
        (
            "scrollbar drag 0.30 to 0.36",
            Gesture::Drag(
                (0..60)
                    .map(|frame| 0.30 + 0.001 * f64::from(frame))
                    .collect(),
            ),
        ),
    ]
}

/// The owner's report: a 150-row C++ file showed "Preparing source highlighting…" after
/// almost every scroll. Its page holds the whole file, coloured as read (the Source viewer
/// knows the lexer state at the file's start), so nothing is prepared, refreshed or noticed,
/// when opened or while scrolling, and no frame shows a row uncoloured. Before, each rest at
/// a new place prepared that place again and showed the notice.
#[test]
fn a_small_source_file_prepares_nothing_and_never_shows_the_notice() {
    let (mut engine, mut window) = source_view(cpp(150));
    assert_eq!(
        (engine.preparations, engine.refreshes, engine.notices),
        (0, 0, 0),
        "opening the file"
    );
    for (name, gesture) in gestures() {
        let work = measure(&mut engine, &mut window, &gesture);
        assert_eq!(work, Work::default(), "{name}");
    }
}

/// A file within one preparation's walk (256 KiB) is prepared whole once per revision: then
/// every page read from it comes coloured, so a jump into its middle and every gesture
/// prepare, refresh and notice nothing more, and no frame shows a row uncoloured.
#[test]
fn a_file_within_one_walk_is_prepared_once_per_revision() {
    let source = cpp(3000);
    assert!(source.len() > 100 * 1024 && source.len() < 256 * 1024);
    let (mut engine, mut window) = source_view(source);
    assert_eq!(
        (engine.preparations, engine.refreshes, engine.notices),
        (1, 0, 0),
        "opening the file prepares it whole"
    );
    window.seek(&mut engine, 0.6);
    rest(&mut engine, &mut window);
    for (name, gesture) in gestures() {
        let work = measure(&mut engine, &mut window, &gesture);
        assert_eq!(work, Work::default(), "{name}");
    }
    assert_eq!(engine.preparations, 1);
    assert_eq!(
        engine.lexer.as_ref().unwrap().plain,
        0,
        "every page came coloured"
    );
}

/// In 2 MiB of C++ a jump far from the lexer's checkpoints shows plain rows, and only those
/// are prepared, at rest, and coloured in place. The walk leaves checkpoints that colour
/// every page the gestures read later, forwards and backwards, so no gesture prepares,
/// refreshes or notices anything and no frame shows a row uncoloured; only the drag, which
/// reads pages far beyond them, prepares the rows it rests on.
#[test]
fn a_large_file_prepares_only_the_rows_it_shows_plain() {
    let source = cpp(56_000);
    assert!(source.len() > 2 * 1024 * 1024);
    let (mut engine, mut window) = source_view(source);
    assert_eq!(
        (engine.preparations, engine.refreshes, engine.notices),
        (0, 0, 0),
        "the opened page comes coloured"
    );
    let plain = engine.lexer.as_ref().unwrap().plain;
    window.seek(&mut engine, 0.3);
    rest(&mut engine, &mut window);
    assert!(
        engine.lexer.as_ref().unwrap().plain > plain,
        "the jump read plain pages"
    );
    assert!(engine.preparations >= 1 && engine.refreshes >= 1);
    assert!(!shows_plain_rows(&engine), "the rows at rest are coloured");
    for (name, gesture) in gestures() {
        let work = measure(&mut engine, &mut window, &gesture);
        if name.starts_with("scrollbar") {
            assert_eq!(work.notices, 0, "{name}: {work:?}");
            assert!(work.preparations <= 4, "{name}: {work:?}");
            assert!(!shows_plain_rows(&engine), "{name}: plain at rest");
        } else {
            assert_eq!(work, Work::default(), "{name}");
        }
    }
}

/// Measurement: preparations, refreshes, notices and frames showing uncoloured rows per
/// gesture, for the small C++ file, 120 KB, 2 MiB and 16 MB of it.
#[test]
#[ignore = "measurement; prints highlighting work per gesture"]
fn measure_highlighting_work_per_gesture() {
    use std::fmt::Write as _;
    let mut report = String::new();
    for (name, rows, seek) in [
        ("C++, 150 rows", 150, None),
        ("C++, 3000 rows", 3000, Some(0.6)),
        ("C++, 56000 rows", 56_000, Some(0.3)),
        ("C++, 400000 rows", 400_000, Some(0.8)),
    ] {
        let (mut engine, mut window) = source_view(cpp(rows));
        let _ = writeln!(
            report,
            "-- {name} ({} bytes): open {} preparations, {} refreshes, {} notices",
            engine.source.len(),
            engine.preparations,
            engine.refreshes,
            engine.notices
        );
        // The page at rest: its bytes and how many screens it holds.
        let extent = |engine: &Engine| {
            let worker = &engine.worker;
            let page = worker.page.as_ref().map_or(0, |page| page.text.len());
            let height = worker.shaped.as_ref().map_or(0.0, |shaped| shaped.height());
            format!(
                "page {} KiB, {:.1} screens",
                page / 1024,
                height / worker.geometry.height as f32
            )
        };
        if let Some(position) = seek {
            let work = measure(&mut engine, &mut window, &Gesture::Drag(vec![position]));
            let _ = writeln!(report, "jump to {position}: {work:?}; {}", extent(&engine));
        }
        for (gesture, inputs) in gestures() {
            let work = measure(&mut engine, &mut window, &inputs);
            let _ = writeln!(report, "{gesture:<28} {work:?}; {}", extent(&engine));
        }
        let lexer = engine.lexer.as_ref().unwrap();
        let _ = writeln!(
            report,
            "pages read highlighted {}, plain {}; walked {} KiB",
            lexer.highlighted,
            lexer.plain,
            lexer.walked / 1024
        );
    }
    println!("{report}");
}

/// The travel of every gesture over the small file is its whole distance or the file's end.
#[test]
fn the_small_file_scrolls_its_whole_distance_while_highlighted() {
    let (mut engine, mut window) = source_view(cpp(150));
    let moved = try_gesture(&mut engine, &mut window, &[(NOTCH, false); 5], false);
    assert!(
        moved.is_some_and(|moved| (moved - 5.0 * NOTCH).abs() < 0.01),
        "{moved:?}"
    );
    let moved = try_gesture(&mut engine, &mut window, &[(-NOTCH, false); 5], false);
    assert!(
        moved.is_some_and(|moved| (moved + 5.0 * NOTCH).abs() < 0.01),
        "{moved:?}"
    );
}
