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
