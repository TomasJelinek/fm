use super::*;
use std::time::SystemTime;

fn session() -> std::sync::Arc<UserCommandOutputSession> {
    UserCommandOutputSession::new(
        UserCommandRequestId(1),
        UserCommandId::new("tool").unwrap(),
        "Tool".into(),
        1,
    )
}

fn texts(rows: &Rows) -> Vec<String> {
    (0..rows.row_count())
        .map(|index| rows.row_data(index).unwrap().text.to_string())
        .collect()
}

fn run(index: u64, exit: Option<ProgramExit>, stopped: bool) -> OutputRun {
    OutputRun {
        index,
        command_line: "tool".into(),
        directory: "/work".into(),
        started: SystemTime::now(),
        process_id: 1,
        exit,
        duration: Some(Duration::from_millis(1200)),
        stopped,
    }
}

#[test]
fn rows_follow_the_session_with_partial_lines_last_and_clear() {
    let session = session();
    let rows = Rows::default();
    session.append(
        OutputStream::Stdout,
        vec!["one".into(), "two".into()],
        Some("thr".into()),
    );
    session.append(OutputStream::Stderr, vec!["oops".into()], None);
    rows.apply(&session.read_since(rows.next()));
    assert_eq!(texts(&rows), ["one", "two", "oops", "thr"]);
    assert!(rows.row_data(3).unwrap().partial);
    assert_eq!(rows.row_data(2).unwrap().kind, 1);
    // The partial line completes; only the new rows arrive.
    session.append(OutputStream::Stdout, vec!["three".into()], None);
    rows.apply(&session.read_since(rows.next()));
    assert_eq!(texts(&rows), ["one", "two", "oops", "three"]);
    assert_eq!(rows.columns(), 5);
    session.clear();
    rows.apply(&session.read_since(rows.next()));
    assert_eq!(rows.row_count(), 0);
    assert_eq!(rows.columns(), 0);
    session.append(OutputStream::Stdout, vec!["after".into()], None);
    rows.apply(&session.read_since(rows.next()));
    assert_eq!(texts(&rows), ["after"]);
}

#[test]
fn rows_drop_at_the_front_as_the_session_does() {
    let session = session();
    let rows = Rows::default();
    session.append(OutputStream::Stdout, vec!["first".into()], None);
    rows.apply(&session.read_since(rows.next()));
    session.append(
        OutputStream::Stdout,
        (0..MAX_OUTPUT_LINES)
            .map(|index| index.to_string())
            .collect(),
        None,
    );
    rows.apply(&session.read_since(rows.next()));
    assert_eq!(rows.row_count(), MAX_OUTPUT_LINES);
    assert_eq!(rows.row_data(0).unwrap().text, "0");
}

#[test]
fn find_wraps_around_in_both_directions_ignoring_case() {
    let session = session();
    let rows = Rows::default();
    session.append(
        OutputStream::Stdout,
        ["Removing a", "keep", "removing b", "keep"]
            .map(String::from)
            .to_vec(),
        None,
    );
    rows.apply(&session.read_since(0));
    assert_eq!(rows.find("REMOVING", -1, false), Some(0));
    assert_eq!(rows.find("removing", 0, false), Some(2));
    assert_eq!(rows.find("removing", 2, false), Some(0), "wraps");
    assert_eq!(rows.find("removing", -1, true), Some(2));
    assert_eq!(rows.find("removing", 0, true), Some(2), "wraps backward");
    assert_eq!(rows.find("absent", -1, false), None);
    assert_eq!(rows.find("", -1, false), None);
}

/// `MAX_OUTPUT_LINES` (50,000) retained lines of build output, some of them not ASCII
/// and one with a Greek capital sigma.
fn full_output() -> Rows {
    let session = session();
    let rows = Rows::default();
    let lines: Vec<String> = (0..MAX_OUTPUT_LINES)
        .map(|line| match line % 1_000 {
            _ if line == 42 => format!("{line}: ΟΔΟΣ ΣΟΦΟΣ"),
            7 => format!("{line}: Ärger mit Straße"),
            _ => format!("{line}: Compiling crate-{line} v0.5.2 (D:\\work\\crates\\c{line})"),
        })
        .collect();
    session.append(OutputStream::Stdout, lines, None);
    rows.apply(&session.read_since(0));
    assert_eq!(rows.row_count(), MAX_OUTPUT_LINES);
    rows
}

/// A Find over every retained line, also one that matches nothing, allocates a fixed
/// few times however many lines it reads (a line with a capital sigma, whose lower case
/// depends on its neighbours, is lower-cased into a string of its own); it used to
/// lower-case each line into a new string, 50,000 allocations for a miss.
#[test]
fn find_reads_every_line_without_allocating_per_line() {
    let rows = full_output();
    for (needle, from, wanted) in [
        ("NOT PRESENT", -1, None),
        ("crate-49999 V0.5.2", -1, Some(49_999)),
        ("compiling crate-3 ", 10, Some(3)),
        ("STRASSE", -1, None),
        ("straße", 8, Some(1_007)),
        ("σοφος", -1, Some(42)),
    ] {
        let (found, allocations) =
            crate::allocation_meter::count(|| rows.find(needle, from, false));
        assert_eq!(found, wanted, "{needle:?}");
        assert!(
            allocations <= 4,
            "{needle:?}: {allocations} allocations over {MAX_OUTPUT_LINES} lines"
        );
    }
}

/// Find ignores case exactly as lower-casing the line and the text does
/// (`str::to_lowercase`): full Unicode mappings that change a character's length, the
/// final sigma, characters that become ASCII, and mixed lines.
#[test]
fn find_ignores_case_as_unicode_lower_casing_does() {
    let session = session();
    let rows = Rows::default();
    let lines = [
        "plain ascii LINE",
        "İstanbul",
        "ΟΔΟΣ ΣΟΦΟΣ",
        "Kelvin 5 K",
        "STRASSE",
        "Straße",
        "ǅemal",
        "ﬀ ligature",
        "",
    ];
    session.append(
        OutputStream::Stdout,
        lines.iter().map(|line| (*line).to_owned()).collect(),
        None,
    );
    rows.apply(&session.read_since(0));
    let needles = [
        "LINE",
        "line",
        "i̇stanbul",
        "İSTANBUL",
        "istanbul",
        "οδος",
        "οδοσ",
        "ΣΟΦΟΣ",
        "σοφος",
        "\u{212A}",
        "k",
        "5 k",
        "strasse",
        "straße",
        "STRAẞE",
        "ǆ",
        "ǅ",
        "ﬀ",
        "ff",
        "Σ",
        "ς",
        " ",
    ];
    for needle in needles {
        let folded = needle.to_lowercase();
        let wanted = lines
            .iter()
            .position(|line| !folded.is_empty() && line.to_lowercase().contains(&folded));
        assert_eq!(rows.find(needle, -1, false), wanted, "{needle:?}");
    }
}

/// What a Find costs over `MAX_OUTPUT_LINES` (50,000) lines: a miss, which reads every
/// line, and the old way (each line lower-cased into a new string) and a cache of
/// lower-cased lines for comparison. Reports timings without thresholds; run alone, in
/// release, with --ignored --nocapture.
#[test]
#[ignore = "explicit command-output Find benchmark; reports timings without pass thresholds"]
fn find_benchmark() {
    use std::time::Instant;
    let rows = full_output();
    let time = |what: &str, find: &dyn Fn() -> Option<usize>| {
        let mut samples: Vec<f64> = (0..9)
            .map(|_| {
                let start = Instant::now();
                assert_eq!(find(), None);
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        let ((), allocations) = crate::allocation_meter::count(|| {
            find();
        });
        println!(
            "{what} over {MAX_OUTPUT_LINES} lines: median {:.2} ms, worst {:.2} ms, \
             {allocations} allocations",
            samples[4], samples[8]
        );
    };
    time("Find (miss)", &|| rows.find("NOT PRESENT", -1, false));
    let texts: Vec<SharedString> = (0..rows.row_count())
        .map(|index| rows.row_data(index).unwrap().text)
        .collect();
    time("lower-case each line (before)", &|| {
        let needle = "NOT PRESENT".to_lowercase();
        texts
            .iter()
            .position(|text| text.to_lowercase().contains(&needle))
    });
    let started = Instant::now();
    let (cache, allocations) = crate::allocation_meter::count(|| {
        texts
            .iter()
            .map(|text| text.to_lowercase())
            .collect::<Vec<_>>()
    });
    let bytes: usize = cache.iter().map(String::capacity).sum();
    println!(
        "cache of lower-cased lines: built in {:.2} ms, {allocations} allocations, \
         {bytes} bytes",
        started.elapsed().as_secs_f64() * 1000.0
    );
    time("search a cache of lower-cased lines", &|| {
        let needle = "NOT PRESENT".to_lowercase();
        cache.iter().position(|text| text.contains(&needle))
    });
}

#[test]
fn status_reads_how_the_runs_ended() {
    let locale = NumberLocale::default();
    let mut view = OutputView {
        planned: 1,
        ..Default::default()
    };
    assert_eq!(status(&view, &locale), ("Running…".into(), false));
    view.stopping = true;
    assert_eq!(status(&view, &locale).0, "Stopping…");
    view.stopping = false;
    view.runs = vec![run(0, Some(ProgramExit::Code(0)), false)];
    view.finished = Some(OutputFinish {
        cancelled: false,
        error: None,
    });
    assert_eq!(
        status(&view, &locale),
        ("Finished · Exit code 0 · 1.2 s".into(), false)
    );
    assert!(succeeded(&view));
    view.runs[0].exit = Some(ProgramExit::Code(2));
    assert_eq!(
        status(&view, &locale),
        ("Failed · Exit code 2 · 1.2 s".into(), true)
    );
    assert!(!succeeded(&view));
    view.runs[0].stopped = true;
    assert!(status(&view, &locale).0.starts_with("Stopped · "));
    // Several runs.
    view.planned = 3;
    view.runs = vec![
        run(0, Some(ProgramExit::Code(0)), false),
        run(1, Some(ProgramExit::Code(0)), false),
        run(2, Some(ProgramExit::Code(0)), false),
    ];
    assert_eq!(
        status(&view, &locale),
        (
            "Finished · 3 of 3 runs · all exited with code 0 · 3.6 s".into(),
            false
        )
    );
    assert!(succeeded(&view));
    view.runs.pop();
    view.finished = Some(OutputFinish {
        cancelled: true,
        error: None,
    });
    assert!(status(&view, &locale).1);
    assert!(!succeeded(&view));
    view.runs.clear();
    view.finished = Some(OutputFinish {
        cancelled: false,
        error: Some("Not started: the program was not found".into()),
    });
    assert_eq!(
        status(&view, &locale),
        ("Not started: the program was not found".into(), true)
    );
    view.finished = None;
    view.runs = vec![run(0, None, false), run(1, None, false)];
    assert_eq!(status(&view, &locale).0, "Running 2 of 3…");
}

#[test]
fn wide_characters_take_two_columns_of_the_scroll_width() {
    assert_eq!(columns("Removing a.o"), 12);
    assert_eq!(columns("文档 x"), 6);
    assert_eq!(columns("한글"), 4);
    assert_eq!(
        columns("e\u{301}"),
        2,
        "a combining mark is counted, never under"
    );
}
