use super::*;
use std::path::PathBuf;

fn lines(input: &[&[u8]]) -> (Vec<String>, String) {
    let mut terminal = TerminalLines::default();
    let mut complete = Vec::new();
    for chunk in input {
        terminal.feed(chunk, |line| complete.push(decode_utf8_lossy(&line)));
    }
    let partial = decode_partial(terminal.partial(), decode_utf8_lossy);
    (complete, partial)
}

#[test]
fn colour_title_and_cursor_sequences_are_removed() {
    let (complete, partial) = lines(&[
        b"\x1b[1;31merror\x1b[0m: bad\n",
        b"\x1b]0;window title\x07plain\n",
        b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\n",
        b"\x1b[2K\x1b[1Gcleared\n\x1b(Bcharset\n",
        b"split \x1b[3",
        b"2mgreen\x1b[",
        b"0m tail",
    ]);
    assert_eq!(
        complete,
        ["error: bad", "plain", "link", "cleared", "charset"]
    );
    assert_eq!(partial, "split green tail");
}

#[test]
fn carriage_return_rewrites_the_line_like_a_progress_counter() {
    let (complete, partial) = lines(&[
        b"Receiving objects:  10% (1/10)\r",
        b"Receiving objects:  50% (5/10)\r",
        b"Receiving objects: 100% (10/10), done.\r\n",
        b"windows line\r\nnext\r",
        b"\rnew",
    ]);
    assert_eq!(
        complete,
        ["Receiving objects: 100% (10/10), done.", "windows line"]
    );
    assert_eq!(partial, "new");
    // A bare carriage return at the end leaves the line until something replaces it.
    let (_, partial) = lines(&[b"50%\r"]);
    assert_eq!(partial, "50%");
}

#[test]
fn backspace_tabs_and_other_controls() {
    let (complete, _) = lines(&[
        "spin|\x08/\x08-\x08done\n".as_bytes(),
        "ab\u{e9}\x08c\n".as_bytes(),
        b"a\tb\tend\n",
        b"bell\x07 nul\x00 del\x7f!\n",
    ]);
    assert_eq!(complete[0], "spindone");
    assert_eq!(
        complete[1], "abc",
        "backspace removes a whole UTF-8 character"
    );
    assert_eq!(expand_tabs(complete[2].clone()), "a       b       end");
    assert_eq!(complete[3], "bell nul del!");
}

#[test]
fn invalid_utf8_is_replaced_and_split_characters_wait_for_their_bytes() {
    let (complete, partial) = lines(&[b"bad \xff\xfe bytes\n", "caf\u{e9} ok".as_bytes()]);
    assert_eq!(complete, ["bad \u{fffd}\u{fffd} bytes"]);
    assert_eq!(partial, "caf\u{e9} ok");
    // The first byte of "é" has arrived, the second has not.
    let (_, partial) = lines(&[b"caf\xc3"]);
    assert_eq!(partial, "caf");
    let (complete, _) = lines(&[b"caf\xc3", b"\xa9\n"]);
    assert_eq!(complete, ["caf\u{e9}"]);
}

#[test]
fn an_endless_line_wraps_at_whole_characters() {
    let mut terminal = TerminalLines::default();
    let mut complete = Vec::new();
    let mut input = vec![b'x'; MAX_OUTPUT_LINE_BYTES - 1];
    input.extend("\u{20ac}".as_bytes()); // three bytes straddling the limit
    input.extend(b"tail");
    terminal.feed(&input, |line| complete.push(line));
    assert_eq!(complete.len(), 1);
    assert_eq!(complete[0].len(), MAX_OUTPUT_LINE_BYTES - 1);
    assert!(std::str::from_utf8(&complete[0]).is_ok());
    assert_eq!(terminal.partial(), "\u{20ac}tail".as_bytes());
    assert_eq!(terminal.finish().unwrap(), "\u{20ac}tail".as_bytes());
    assert!(terminal.finish().is_none());
}

#[test]
fn log_drops_the_oldest_lines_beyond_its_bounds() {
    let mut log = OutputLog::default();
    for index in 0..MAX_OUTPUT_LINES + 10 {
        log.push(OutputLine {
            stream: OutputStream::Stdout,
            text: index.to_string(),
        });
    }
    assert_eq!(log.len(), MAX_OUTPUT_LINES);
    assert_eq!(log.dropped(), 10);
    assert_eq!(log.first(), 10);
    assert_eq!(log.next(), MAX_OUTPUT_LINES as u64 + 10);
    assert_eq!(log.since(0).next().unwrap().text, "10");
    assert_eq!(
        log.since(log.next() - 2)
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        [
            (MAX_OUTPUT_LINES + 8).to_string(),
            (MAX_OUTPUT_LINES + 9).to_string()
        ]
    );
    // The byte bound: a few huge lines.
    let mut log = OutputLog::default();
    let big = "y".repeat(MAX_OUTPUT_LINE_BYTES);
    for _ in 0..(MAX_OUTPUT_BYTES / MAX_OUTPUT_LINE_BYTES + 5) {
        log.push(OutputLine {
            stream: OutputStream::Stderr,
            text: big.clone(),
        });
    }
    assert!(log.len() * MAX_OUTPUT_LINE_BYTES <= MAX_OUTPUT_BYTES);
    assert_eq!(log.dropped(), 5);
    log.set_partial(OutputStream::Stdout, Some("still writing".into()));
    log.clear();
    assert!(log.is_empty());
    assert_eq!(log.dropped(), 0);
    assert_eq!(log.first(), log.next());
    assert_eq!(
        log.text(),
        "still writing\n",
        "Clear keeps the line being written"
    );
}

#[test]
fn session_notifies_once_until_read_and_reports_runs() {
    let session = UserCommandOutputSession::new(
        UserCommandRequestId(7),
        UserCommandId::new("git-clean").unwrap(),
        "git clean".into(),
        1,
    );
    assert!(!session.take_notification(), "nothing changed yet");
    session.begin_run(OutputRun {
        index: 0,
        command_line: "git.exe clean -fxd".into(),
        directory: "C:\\work".into(),
        started: SystemTime::now(),
        process_id: 42,
        exit: None,
        duration: None,
        stopped: false,
    });
    session.append(
        OutputStream::Stdout,
        vec!["Removing a.o".into()],
        Some("Removing b".into()),
    );
    assert!(session.take_notification());
    session.append(OutputStream::Stderr, vec!["warning".into()], None);
    assert!(
        !session.take_notification(),
        "one notification until the UI reads"
    );
    let view = session.read_since(0);
    assert_eq!(
        view.lines
            .iter()
            .map(|line| (line.stream, line.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (OutputStream::Notice, "▶ git.exe clean -fxd"),
            (OutputStream::Stdout, "Removing a.o"),
            (OutputStream::Stderr, "warning"),
        ]
    );
    assert_eq!(view.partial.len(), 1);
    assert_eq!(view.next, 3);
    assert!(
        session.take_notification(),
        "the change before the read is still announced"
    );
    let _ = session.read_since(view.next);
    assert!(!session.take_notification());
    session.request_stop();
    assert!(session.read_since(3).stopping);
    session.end_run(0, Some(ProgramExit::Code(1)), Duration::from_millis(1500));
    session.finish(true, None);
    let view = session.read_since(3);
    assert!(!view.stopping);
    assert_eq!(view.lines[0].text, "Stopped · Exit code 1 · 1.5 s");
    assert_eq!(view.runs[0].exit, Some(ProgramExit::Code(1)));
    assert!(view.runs[0].stopped);
    assert!(view.finished.unwrap().cancelled);
    assert!(session.text().contains("Removing a.o\nwarning\n"));
    session.clear();
    let view = session.read_since(4);
    assert_eq!(view.first, view.next);
    assert!(view.lines.is_empty());
}

#[test]
fn command_lines_are_shown_with_display_quotes_only() {
    let program = PathBuf::from(r"C:\Program Files\Git\bin\git.exe");
    let arguments: Vec<OsString> = ["clean", "-fxd", "", "two words", "say \"hi\"", r"dir\"]
        .into_iter()
        .map(OsString::from)
        .collect();
    assert_eq!(
        display_command_line(&program, &arguments),
        r#""C:\Program Files\Git\bin\git.exe" clean -fxd "" "two words" "say \"hi\"" dir\"#
    );
    assert_eq!(
        display_command_line(&program, &[OsString::from(r"a b\")]),
        r#""C:\Program Files\Git\bin\git.exe" "a b\\""#
    );
}

#[test]
fn exits_and_durations_read_naturally() {
    assert!(ProgramExit::Code(0).success());
    assert!(!ProgramExit::Code(1).success());
    assert!(!ProgramExit::Signal(9).success());
    assert_eq!(ProgramExit::Code(2).describe(), "Exit code 2");
    assert_eq!(
        ProgramExit::Code(0xC000_013A).describe(),
        "Exit code 3221225786 (0xC000013A)"
    );
    assert_eq!(ProgramExit::Signal(15).describe(), "Ended by signal 15");
    assert_eq!(describe_duration(Duration::from_millis(250)), "0.2 s");
    assert_eq!(describe_duration(Duration::from_secs(125)), "2 min 05 s");
    assert_eq!(describe_duration(Duration::from_secs(3720)), "1 h 02 min");
}
