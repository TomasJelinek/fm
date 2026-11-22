//! Opt-in timing evidence for search and viewport hot paths. Run alone, in the
//! release profile: `cargo test --release --test throughput -- --ignored --nocapture`.
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use std::time::Instant;
use support::Host;

const MIB: f64 = 1024.0 * 1024.0;

fn rust_lines(total: usize) -> String {
    let mut text = String::with_capacity(total + 128);
    let mut n = 0;
    while text.len() < total {
        text.push_str(&format!(
            "    let value_{n:08} = compute(&items[{n}], \"text {n}\"); // note {n}\n"
        ));
        n += 1;
    }
    text
}

#[test]
#[ignore = "timing evidence; run in release with --ignored --nocapture"]
fn search_and_viewport_throughput() {
    let input = rust_lines(24 << 20);
    let length = input.len() as f64;
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_SOURCE,
            display_name: "throughput.rs".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    for case_sensitive in [true, false] {
        let reads = host.reads().len();
        let polls = host.cancel_polls();
        let started = Instant::now();
        let id = host.submit(
            abi::REQUEST_SEARCH,
            &Search {
                query: "absent needle".into(),
                source_anchor: 0,
                backwards: false,
                case_sensitive,
            },
        );
        let events = host.finish();
        let seconds = started.elapsed().as_secs_f64();
        assert!(
            events
                .iter()
                .any(|e| e.id == id && e.kind == abi::EVENT_COMPLETED)
        );
        let requested: u64 = host.reads()[reads..]
            .iter()
            .map(|read| u64::from(read.length))
            .sum();
        println!(
            "search case_sensitive={case_sensitive} mib_per_s={:.1} requested_per_byte={:.2} polls_per_mib={:.1}",
            length / MIB / seconds,
            requested as f64 / length,
            (host.cancel_polls() - polls) as f64 / (length / MIB),
        );
    }
    let polls = host.cancel_polls();
    let started = Instant::now();
    let (mut anchor, mut pages) = (0, 0);
    while anchor < 4 << 20 {
        let id = host.submit(
            abi::REQUEST_VIEWPORT,
            &Viewport {
                source_anchor: anchor,
                backwards: false,
                max_bytes: 12 * 1024,
                max_lines: 60,
            },
        );
        let events = host.finish();
        let page: TextPage = codec::decode(
            &events
                .iter()
                .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
                .expect("page")
                .payload,
        )
        .unwrap();
        assert!(!page.spans.is_empty());
        anchor = page.source.end().unwrap();
        pages += 1;
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "viewport pages={pages} us_per_page={:.1} source_mib_per_s={:.1} polls_per_page={:.1}",
        seconds * 1e6 / f64::from(pages),
        anchor as f64 / MIB / seconds,
        (host.cancel_polls() - polls) as f64 / f64::from(pages),
    );
}

/// Synthetic C++ in the shape of ordinary library code: preprocessor lines,
/// namespaces, templates, classes, standard types, calls, literals and comments.
fn cpp_lines(total: usize) -> String {
    let mut text = String::with_capacity(total + 1024);
    let mut n = 0usize;
    while text.len() < total {
        text.push_str(&format!(
            "#include <vector>
#if defined(BENCH_{n}) && !defined(NDEBUG)
// Block {n}: synthetic declarations for lexer timing.
namespace bench_{n} {{
template <typename T, class Alloc = std::allocator<T>>
class Widget{n} final : public Base {{
public:
    explicit Widget{n}(std::size_t count) : items_(count), name_(\"widget_{n}\") {{}}
    std::vector<std::string> names() const noexcept {{ return {{name_, \"beta\"}}; }}
    static constexpr unsigned long long mask = 0x{n:x}ull;
    /* weight {n} */ double weight(int x, uint32_t y) const {{ return x * 1'000 + y / 2.5e3f; }}
    bool contains(const T& value) const {{ return std::find(items_.begin(), items_.end(), value) != items_.end(); }}
private:
    std::unordered_map<int, double> items_;
    std::string name_;
}};
}} // namespace bench_{n}
#endif
"
        ));
        n += 1;
    }
    text
}

/// The median, slowest and fastest of `runs` timings of `lex` in decimal
/// megabytes per second, and the span count of the last run.
fn mb_per_s(bytes: usize, runs: usize, mut lex: impl FnMut() -> usize) -> (f64, f64, f64, usize) {
    let mut rates = Vec::with_capacity(runs);
    let mut spans = 0;
    for _ in 0..runs {
        let started = Instant::now();
        spans = lex();
        rates.push(bytes as f64 / 1e6 / started.elapsed().as_secs_f64());
    }
    rates.sort_by(f64::total_cmp);
    (rates[runs / 2], rates[0], rates[runs - 1], spans)
}

#[test]
#[ignore = "timing evidence; run in release with --ignored --nocapture"]
fn cpp_lex_throughput() {
    use midnight_viewer_source::syntax::{self, Language, State};
    let input = cpp_lines(8 << 20);
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    // One call and one style collector per complete line: the worst case.
    let (median, low, high, spans) = mb_per_s(input.len(), 9, || {
        let mut state = State::default();
        let mut spans = 0;
        for line in &lines {
            let lexed = syntax::highlight(line, Language::Cpp, &mut state, || false).unwrap();
            spans += lexed.spans.len();
        }
        spans
    });
    println!(
        "lex cpp per_line mb_per_s={median:.1} min={low:.1} max={high:.1} bytes={} spans={spans}",
        input.len()
    );
    let chunks = line_chunks(&input);
    let (median, low, high, spans) = mb_per_s(input.len(), 9, || {
        let mut state = State::default();
        let mut spans = 0;
        for chunk in &chunks {
            let lexed = syntax::highlight(chunk, Language::Cpp, &mut state, || false).unwrap();
            assert!(!lexed.truncated, "a chunk's spans fit one page");
            spans += lexed.spans.len();
        }
        spans
    });
    println!(
        "lex cpp chunks_32k mb_per_s={median:.1} min={low:.1} max={high:.1} bytes={} spans={spans}",
        input.len()
    );
}

/// Chunks of at most 32 KiB that end after a newline: one style collector each, as
/// a page's replay and a preparation turn lex a bounded run of complete lines.
fn line_chunks(input: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < input.len() {
        let mut end = (start + 32 * 1024).min(input.len());
        if end < input.len() {
            end = start + input[start..end].rfind('\n').unwrap() + 1;
        }
        chunks.push(&input[start..end]);
        start = end;
    }
    chunks
}

#[test]
#[ignore = "timing evidence; run in release with --ignored --nocapture"]
fn decode_and_lex_throughput() {
    use midnight_viewer_source::{
        decode::{Encoding, decode},
        syntax::{self, Language, State},
    };
    let input = rust_lines(4 << 20);
    let started = Instant::now();
    let mut scalars = 0;
    for chunk in input.as_bytes().chunks(32768) {
        let decoded = decode(chunk, 0, Encoding::Utf8, true, 65536, usize::MAX, || false).unwrap();
        scalars += decoded.anchors.len();
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "decode mib_per_s={:.1} scalars={scalars}",
        input.len() as f64 / MIB / seconds
    );
    let started = Instant::now();
    let mut state = State::default();
    let mut spans = 0;
    for line in input.split_inclusive('\n') {
        let lexed = syntax::highlight(line, Language::Rust, &mut state, || false).unwrap();
        spans += lexed.spans.len();
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "lex rust mib_per_s={:.1} spans={spans}",
        input.len() as f64 / MIB / seconds
    );
    // The same input as medians of nine passes, per line and per 32 KiB chunk.
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    for (mode, pieces) in [("per_line", lines), ("chunks_32k", line_chunks(&input))] {
        let (median, low, high, spans) = mb_per_s(input.len(), 9, || {
            let mut state = State::default();
            let mut spans = 0;
            for piece in &pieces {
                let lexed = syntax::highlight(piece, Language::Rust, &mut state, || false).unwrap();
                assert!(!lexed.truncated, "a chunk's spans fit one page");
                spans += lexed.spans.len();
            }
            spans
        });
        println!("lex rust {mode} mb_per_s={median:.1} min={low:.1} max={high:.1} spans={spans}");
    }
}
