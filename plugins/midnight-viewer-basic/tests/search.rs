#[allow(dead_code)]
mod harness;
use harness::Host;
use midnight_plugin_sdk::api::{abi, codec, messages, viewer};
use std::time::Instant;

fn search(
    h: &mut Host,
    id: u64,
    query: &str,
    anchor: u64,
    backwards: bool,
    case_sensitive: bool,
) -> Option<viewer::SourceRange> {
    h.submit(
        id,
        1,
        1,
        abi::REQUEST_SEARCH,
        &messages::Search {
            query: query.into(),
            source_anchor: anchor,
            backwards,
            case_sensitive,
        },
    );
    h.pump();
    if let Some(event) = h
        .events
        .iter()
        .find(|e| e.id == id && e.kind == abi::EVENT_SEARCH_MATCH)
    {
        let found: messages::SearchMatch = codec::decode(&event.payload).unwrap();
        return Some(found.source);
    }
    h.event(id, abi::EVENT_COMPLETED);
    None
}

fn range(offset: usize, length: usize) -> viewer::SourceRange {
    viewer::SourceRange {
        offset: offset as u64,
        length: length as u64,
    }
}

#[test]
fn forward_search_reads_large_contiguous_windows_and_polls_cancellation_sparingly() {
    let mut h = Host::new(vec![b'x'; 1 << 20]);
    h.open(viewer::MODE_TEXT);
    let (reads, polls) = (h.reads.len(), h.polls());
    assert_eq!(search(&mut h, 2, "needle", 0, false, true), None);
    let reads = &h.reads[reads..];
    assert!(reads.len() <= 8, "{} reads for 1 MiB", reads.len());
    assert_eq!(reads[0].offset, 0);
    for pair in reads.windows(2) {
        assert_eq!(pair[1].offset, pair[0].offset + u64::from(pair[0].length));
    }
    let polls = h.polls() - polls;
    assert!(polls <= 1024, "{polls} host cancellation polls for 1 MiB");
}

#[test]
fn backward_search_reads_only_windows_near_its_anchor() {
    let mut h = Host::new(b"--needle--".to_vec());
    h.base = 16 << 20;
    h.open(viewer::MODE_TEXT);
    let base = h.base as usize;
    let before = h.reads.len();
    assert_eq!(
        search(&mut h, 2, "needle", base as u64 + 10, true, true),
        Some(range(base + 2, 6))
    );
    let requested: u64 = h.reads[before..]
        .iter()
        .map(|read| u64::from(read.length))
        .sum();
    assert!(requested <= 512 * 1024, "{requested} bytes requested");
}

#[test]
fn chunked_backward_and_forward_search_agree_with_a_whole_text_scan() {
    // Mixed-width UTF-8 with CRLF lines; needles are separated by gaps around the
    // 256 KiB search window so matches fall on both sides of window edges.
    let line = "Příliš žluťoučký kůň úpěl ďábelské ódy, 中文 😀\tCRLF\r\n";
    let mut text = String::new();
    for gap in [1_000, 10, 70_000, 262_100, 262_150, 4] {
        let previous_end = text.len();
        while text.len() < previous_end + gap {
            text.push_str(line);
        }
        text.push_str("needle");
    }
    text.push_str(&line.repeat(200));
    let needles: Vec<usize> = text.match_indices("needle").map(|(at, _)| at).collect();
    let mut anchors = vec![0, text.len()];
    for &at in &needles {
        for delta in [0, 1, 5, 6, 7, 1_000, 262_143, 262_144, 262_150] {
            anchors.extend([at + delta, at.saturating_sub(delta)]);
        }
    }
    anchors.retain(|&anchor| anchor <= text.len());
    anchors.sort_unstable();
    anchors.dedup();
    let mut h = Host::new(text.clone().into_bytes());
    h.open(viewer::MODE_TEXT);
    let mut id = 2;
    for (index, &anchor) in anchors.iter().enumerate() {
        let before = needles.iter().rev().find(|&&at| at + 6 <= anchor);
        let after = needles.iter().find(|&&at| at >= anchor);
        let (query, sensitive) = if index % 3 == 0 {
            ("NeEdLe", false)
        } else {
            ("needle", true)
        };
        assert_eq!(
            search(&mut h, id, query, anchor as u64, true, sensitive),
            before.map(|&at| range(at, 6)),
            "backward from {anchor}"
        );
        assert_eq!(
            search(&mut h, id + 1, query, anchor as u64, false, sensitive),
            after.map(|&at| range(at, 6)),
            "forward from {anchor}"
        );
        id += 2;
    }
}

#[test]
fn search_windows_starting_inside_scalars_never_match_resynchronization_escapes() {
    // No real invalid bytes exist, so escapes produced while resynchronizing after a
    // read that starts inside "中" (E4 B8 AD) must never be reported.
    let text = "中".repeat(150_000);
    let mut h = Host::new(text.into_bytes());
    h.open(viewer::MODE_TEXT);
    let mut id = 2;
    for anchor in [450_000, 449_999, 262_145, 262_146, 100_001] {
        for query in ["\\xAD", "\\xB8\\xAD"] {
            assert_eq!(search(&mut h, id, query, anchor, true, true), None);
            assert_eq!(search(&mut h, id + 1, query, anchor, false, true), None);
            id += 2;
        }
    }
}

#[test]
#[ignore = "timing: cargo test --release --test search -- --ignored --nocapture"]
fn search_throughput() {
    for (name, line) in [
        (
            "ascii",
            "The quick brown fox jumps over the lazy dog 0123456789.\n",
        ),
        (
            "czech",
            "Příliš žluťoučký kůň úpěl ďábelské ódy na louce.\n",
        ),
        ("cjk", "中文字符测试数据行，包含标点符号和数字一二三。\n"),
    ] {
        let data = line.repeat((16 << 20) / line.len()).into_bytes();
        let size = data.len();
        for sensitive in [true, false] {
            let mut h = Host::new(data.clone());
            h.open(viewer::MODE_TEXT);
            let (reads, polls) = (h.reads.len(), h.polls());
            let started = Instant::now();
            assert_eq!(
                search(&mut h, 2, "absent needle", 0, false, sensitive),
                None
            );
            let seconds = started.elapsed().as_secs_f64();
            println!(
                "basic search {name} sensitive={sensitive} bytes={size} mib_per_s={:.1} reads={} polls={}",
                size as f64 / f64::from(1 << 20) / seconds,
                h.reads.len() - reads,
                h.polls() - polls
            );
        }
    }
}

#[test]
#[ignore = "timing: cargo test --release --test search -- --ignored --nocapture"]
fn repeated_backward_search_timing() {
    // 32 MiB; the last MiB has a match every 4 KiB. Five Shift+F3 presses from the end.
    let mut data = vec![b'x'; 31 << 20];
    let mut block = vec![b'y'; 4090];
    block.extend_from_slice(b"needle");
    data.extend(block.repeat(256));
    let mut h = Host::new(data.clone());
    h.open(viewer::MODE_TEXT);
    let (reads, started) = (h.reads.len(), Instant::now());
    let mut anchor = data.len() as u64;
    for press in 0..5 {
        let found = search(&mut h, 2 + press, "needle", anchor, true, true).unwrap();
        let distance = if press == 0 { 6 } else { 4096 };
        assert_eq!(found.offset, anchor - distance);
        anchor = found.offset;
    }
    let requested: u64 = h.reads[reads..]
        .iter()
        .map(|read| u64::from(read.length))
        .sum();
    println!(
        "basic backward search presses=5 bytes={} ms={:.1} requested_bytes={requested}",
        data.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
}
