//! Sustained scrolling of a mid-sized C++ source file in the Source view at Chromium's wheel
//! speed: the timed co-simulation of `worker_prepared_tests.rs` over a synthetic file shaped
//! like a real API facade (about 1,560 rows and 76 KB of space-indented, ASCII C++: short
//! forwarding functions whose long parameter lists continue on rows aligned under their
//! opening parenthesis, every row at most 120 bytes).
use super::frontier::{Engine, Window, settle};
use super::highlight::{Lexer, highlight_cpp};
use super::prepared::{Input, Script, every, geometry, percentile, run, scripts};
use super::*;
use std::fmt::Write as _;

const TYPES: [&str; 8] = [
    "StatusValue",
    "const CatalogRecord&",
    "std::uint32_t",
    "const LayoutOptions&",
    "ReaderAccountHandle",
    "double",
    "const char*",
    "LoanHistoryBuffer*",
];
const VERBS: [&str; 8] = [
    "open", "close", "renew", "lookup", "reserve", "archive", "import", "export",
];
const NOUNS: [&str; 10] = [
    "CatalogRecord",
    "ReaderAccount",
    "ShelfLayoutOptions",
    "PublisherIndex",
    "LoanHistoryEntries",
    "BranchOpeningHours",
    "AcquisitionBudget",
    "Subscription",
    "TransferQueue",
    "BarcodeLabelBatch",
];
const PARAMETERS: [&str; 9] = [
    "catalogHandle",
    "options",
    "recordCount",
    "outputBuffer",
    "branchIndex",
    "lateFeeInCents",
    "callerContext",
    "historyCapacity",
    "layoutOptions",
];

/// Synthetic C++ with the aggregate shape of a real API facade of about 76 KB: per
/// function, a comment now and then, a signature indented four spaces whose parameters
/// continue on rows aligned under its parenthesis, a brace, a `return` that forwards the
/// call (wrapped when long), a closing brace and a blank row. Deterministic.
pub(super) fn api_cpp(functions: usize) -> String {
    let mut text = String::from("#include \"library_facade.h\"\n#include <cstdint>\n\n");
    text.push_str("namespace library\n{\n\n");
    let mut seed: u32 = 0x9e37_79b9;
    let mut next = |limit: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as usize % limit
    };
    for function in 0..functions {
        if function % 4 == 0 {
            let _ = writeln!(
                text,
                "    // Forwards the request to the implementation; see the interface for details."
            );
        }
        let name = format!("{}{}", VERBS[next(VERBS.len())], NOUNS[next(NOUNS.len())]);
        let count = 1 + next(5);
        let parameters: Vec<String> = (0..count)
            .map(|index| {
                format!(
                    "{} {}{}",
                    TYPES[next(TYPES.len())],
                    PARAMETERS[next(PARAMETERS.len())],
                    index
                )
            })
            .collect();
        let head = format!("    StatusValue LibraryFacade::{name}(");
        let indent = " ".repeat(head.len());
        let mut row = head.clone();
        for (index, parameter) in parameters.iter().enumerate() {
            let tail = if index + 1 == count { ")" } else { "," };
            if row.len() > head.len() && row.len() + 1 + parameter.len() + tail.len() > 118 {
                text.push_str(&row);
                text.push('\n');
                row = indent.clone();
            } else if row.len() > head.len() {
                row.push(' ');
            }
            row.push_str(parameter);
            row.push_str(tail);
        }
        text.push_str(&row);
        text.push_str("\n    {\n");
        let arguments: Vec<&str> = parameters
            .iter()
            .map(|parameter| parameter.rsplit(' ').next().unwrap_or(""))
            .collect();
        let call = format!("        return m_service->{name}(");
        let continuation = " ".repeat(call.len());
        let mut row = call.clone();
        for (index, argument) in arguments.iter().enumerate() {
            let tail = if index + 1 == count { ");" } else { "," };
            if row.len() > call.len() && row.len() + 1 + argument.len() + tail.len() > 118 {
                text.push_str(&row);
                text.push('\n');
                row = continuation.clone();
            } else if row.len() > call.len() {
                row.push(' ');
            }
            row.push_str(argument);
            row.push_str(tail);
        }
        text.push_str(&row);
        text.push_str("\n    }\n\n");
    }
    text.push_str("} // namespace library\n");
    text
}

/// The synthetic file at the measured size: about 1,560 rows and 76 KB.
pub(super) fn api_fixture() -> String {
    api_cpp(229)
}

#[test]
fn the_synthetic_fixture_has_the_measured_shape() {
    let text = api_fixture();
    let rows: Vec<&str> = text.lines().collect();
    let mut lengths: Vec<usize> = rows.iter().map(|row| row.len()).collect();
    lengths.sort_unstable();
    let median = lengths[lengths.len() / 2];
    assert!(text.is_ascii() && !text.contains('\t') && !text.contains('\r'));
    assert!(
        (70_000..=82_000).contains(&text.len()),
        "{} bytes",
        text.len()
    );
    assert!((1_450..=1_700).contains(&rows.len()), "{} rows", rows.len());
    assert!(*lengths.last().unwrap() <= 120, "{lengths:?}");
    assert!((40..=70).contains(&median), "median row {median} bytes");
}

/// A Source view of `source` through the lexical model at `size`, at rest at its start.
fn view(source: String, size: ViewerGeometry, delay: u32) -> (Engine, Window) {
    let mut engine = Engine::open(source, viewer::MODE_SOURCE, size, delay).highlighted();
    engine.lexer = Some(Lexer::new());
    engine.highlight_replies = Some(highlight_cpp);
    let mut window = Window::default();
    assert!(settle(&mut engine, &mut window));
    (engine, window)
}

/// Sustained gestures at Chromium's wheel speed: a steady wheel of 20 notches a second for
/// three seconds and a fast spin of 60 a second for a second, down and back up, and a
/// scrollbar drag over the whole file in one second, there and back.
fn sustained(notch: f64) -> Vec<Script> {
    vec![
        Script {
            name: "steady wheel down, 20/s",
            frames: every(60, 3, Input::Wheel(notch)),
        },
        Script {
            name: "steady wheel up, 20/s",
            frames: every(60, 3, Input::Wheel(-notch)),
        },
        Script {
            name: "fast spin down, 60/s",
            frames: every(60, 1, Input::Wheel(notch)),
        },
        Script {
            name: "fast spin up, 60/s",
            frames: every(60, 1, Input::Wheel(-notch)),
        },
        Script {
            name: "scrollbar drag 0 to 1 in 1 s",
            frames: (0..=60)
                .map(|frame| vec![Input::Position(frame as f64 / 60.0)])
                .collect(),
        },
        Script {
            name: "scrollbar drag 1 to 0 in 1 s",
            frames: (0..=60)
                .map(|frame| vec![Input::Position(1.0 - frame as f64 / 60.0)])
                .collect(),
        },
    ]
}

/// Measurement: the synthetic API facade (and, when `SOURCE_SCROLL_FILE` names a file, that
/// file; only aggregate numbers are printed) in the Source view at 1x and 2x under the
/// prepared-window gestures and sustained gestures at Chromium's notch (100 logical px,
/// `PREPARED_NOTCH`), the plugin answering after `SOURCE_SCROLL_DELAY` frames: stalls, lag,
/// the worker's turns, reads and highlighting work.
#[test]
#[ignore = "measurement; prints sustained Source scrolling of a mid-sized C++ file"]
fn measure_source_api_scrolling() {
    let variable = |name: &str| std::env::var(name).ok();
    let notch: f64 = variable("PREPARED_NOTCH")
        .and_then(|value| value.parse().ok())
        .unwrap_or(100.0);
    // Frames the simulated plugin takes to answer a page (`SOURCE_SCROLL_DELAY`, default 1).
    let delay: u32 = variable("SOURCE_SCROLL_DELAY")
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let mut files = vec![("synthetic API facade", api_fixture())];
    if let Some(path) = std::env::var_os("SOURCE_SCROLL_FILE")
        && let Ok(bytes) = std::fs::read(path)
    {
        files.push((
            "named file",
            String::from_utf8_lossy(&bytes).replace("\r\n", "\n"),
        ));
    }
    let mut report = String::new();
    for (name, source) in files {
        for (scale, size) in [
            (1.0, geometry(1312, 720, 1.0)),
            (2.0, geometry(2624, 1440, 2.0)),
        ] {
            let (mut engine, mut window) = view(source.clone(), size, delay);
            let _ = writeln!(
                report,
                "-- {name} ({} bytes, {} rows) at {scale}x, notch {notch} px, delay {delay}",
                source.len(),
                source.lines().count()
            );
            let mut gestures = scripts(notch * scale, f64::from(size.height));
            gestures.extend(sustained(notch * scale));
            let mut stalls = 0;
            let mut turns = Vec::new();
            for script in gestures {
                let measured = run(&mut engine, &mut window, &script);
                stalls += measured.stalls;
                turns.extend_from_slice(&measured.turns);
                let _ = writeln!(report, "{}", measured.line(script.name));
            }
            let _ = writeln!(
                report,
                "   all gestures: stalls {stalls}, turns {} p50/p95/max {:.2}/{:.2}/{:.2} ms",
                turns.len(),
                percentile(&turns, 50),
                percentile(&turns, 95),
                percentile(&turns, 100)
            );
        }
    }
    println!("{report}");
}
