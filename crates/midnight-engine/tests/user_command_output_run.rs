//! Output window runs end to end: the engine, the native adapter and a real program (a
//! rerun of this test binary, reading its own arguments after the first `--`).
use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

const FIXTURE: &str = "user_command_output_run::program_fixture";
const DEADLINE: Duration = Duration::from_secs(60);

#[test]
#[ignore = "program run by the output window tests"]
fn program_fixture() {
    let all: Vec<String> = std::env::args().collect();
    let at = all.iter().position(|argument| argument == "--").unwrap();
    let rest = &all[at + 1..];
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    match rest[0].as_str() {
        "mixed" => {
            // libtest's "test … " line has no line break yet.
            out.write_all(b"\nalpha\n\x1b[1;31mred\x1b[0m text\n")
                .unwrap();
            out.write_all(b"copying  10%\rcopying  50%\rcopying 100%\n")
                .unwrap();
            out.write_all(b"bad \xff byte\n").unwrap();
            out.flush().unwrap();
            err.write_all(b"oops: something\n").unwrap();
            err.flush().unwrap();
            out.write_all(format!("item {}\n", rest.get(1).map_or("", String::as_str)).as_bytes())
                .unwrap();
            out.flush().unwrap();
            std::process::exit(4);
        }
        "sleep" => {
            writeln!(out, "\nsleeping").unwrap();
            out.flush().unwrap();
            std::thread::sleep(DEADLINE);
        }
        other => panic!("unknown mode {other}"),
    }
}

fn command(mode: &str, launch: UserCommandLaunch, extra: &[&str]) -> UserCommand {
    let mut arguments: Vec<String> = [
        "--exact",
        FIXTURE,
        "--ignored",
        "--nocapture",
        "--test-threads=1",
        "--",
        mode,
    ]
    .into_iter()
    .chain(extra.iter().copied())
    .map(str::to_owned)
    .collect();
    arguments.retain(|argument| !argument.is_empty());
    UserCommand {
        id: UserCommandId::new("tool").unwrap(),
        name: "Tool".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments,
        },
        output: UserCommandOutput::NEW_PROGRAM,
    }
}

fn request(id: u64, command: UserCommand, directory: &Path, files: &[&str]) -> UserCommandRequest {
    let entries: Vec<Entry> = files
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(EntryId(index as u64), directory.join(name), EntryKind::File)
        })
        .collect();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let bits = if files.is_empty() {
        vec![]
    } else {
        vec![(1u64 << files.len()) - 1]
    };
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap());
    UserCommandRequest {
        captured: None,
        id: UserCommandRequestId(id),
        command: command.id.clone(),
        configuration: Arc::new(vec![command]),
        context: UserCommandContext {
            active: Some(directory.to_owned()),
            other: None,
            files: (!files.is_empty()).then_some(SelectedFiles {
                selection,
                view,
                fallback: None,
            }),
            focus: None,
        },
    }
}

struct Running {
    send: midnight_engine::CommandSender,
    events: mpsc::Receiver<Event>,
    engine: tokio::task::JoinHandle<()>,
}

fn start() -> Running {
    let service = Arc::new(LocalFileSystem::new(None));
    let (events, receive) = mpsc::channel(64);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        service.clone(),
        events,
    );
    let (send, commands) = engine.command_channel(16).unwrap();
    engine
        .configure_audited_transfers()
        .unwrap()
        .install(service.clone());
    engine.configure_user_commands().unwrap().install(service);
    Running {
        send,
        events: receive,
        engine: tokio::spawn(engine.run(commands)),
    }
}

/// Events until the session opens; the job's updates are collected.
async fn opened(running: &mut Running) -> Arc<UserCommandOutputSession> {
    tokio::time::timeout(DEADLINE, async {
        loop {
            match running.events.recv().await.unwrap() {
                Event::UserCommandOutput(event) if event.opened => return event.session,
                Event::UserCommand(event) => {
                    if let UserCommandUpdate::Finished(summary) = &event.update {
                        panic!("finished before an output window opened: {summary:?}");
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

/// Read the session as the UI does, on each notification, until the job finished.
async fn finished(
    running: &mut Running,
    session: &Arc<UserCommandOutputSession>,
) -> (Vec<OutputLine>, OutputView, UserCommandSummary) {
    let mut lines = Vec::new();
    let mut next = 0;
    let mut last = OutputView::default();
    let summary = tokio::time::timeout(DEADLINE, async {
        loop {
            match running.events.recv().await.unwrap() {
                Event::UserCommandOutput(event) => {
                    assert!(!event.opened && Arc::ptr_eq(&event.session, session));
                    last = session.read_since(next);
                    next = last.next;
                    lines.extend(last.lines.iter().cloned());
                }
                Event::UserCommand(event) => {
                    if let UserCommandUpdate::Finished(summary) = event.update {
                        return summary;
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    let view = session.read_since(next);
    lines.extend(view.lines.iter().cloned());
    (lines, view, summary)
}

fn texts(lines: &[OutputLine], stream: OutputStream) -> Vec<&str> {
    lines
        .iter()
        .filter(|line| line.stream == stream)
        .map(|line| line.text.as_str())
        .filter(|text| !text.starts_with("running ") && !text.is_empty())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_output_window_streams_clean_lines_and_reports_each_items_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["first.txt", "second.txt"] {
        fs::write(directory.path().join(name), name).unwrap();
    }
    let mut running = start();
    let tool = command("mixed", UserCommandLaunch::PerItem, &["${file.name}"]);
    running
        .send
        .send(Command::ExecuteUserCommand(Box::new(request(
            1,
            tool,
            directory.path(),
            &["first.txt", "second.txt"],
        ))))
        .await
        .unwrap();
    let session = opened(&mut running).await;
    assert_eq!(session.name, "Tool");
    let (lines, view, summary) = finished(&mut running, &session).await;
    assert_eq!((summary.planned, summary.started), (2, 2));
    assert!(!summary.cancelled && summary.error.is_none());
    // One run after the other: each item's output between its own notices.
    let notices = texts(&lines, OutputStream::Notice);
    assert_eq!(notices.len(), 4, "{notices:?}");
    assert!(notices[0].starts_with("▶ [1/2] "), "{notices:?}");
    assert!(notices[1].starts_with("Exit code 4 · "), "{notices:?}");
    assert!(notices[2].starts_with("▶ [2/2] "), "{notices:?}");
    let stdout = texts(&lines, OutputStream::Stdout);
    for expected in [
        "alpha",
        "red text",
        "copying 100%",
        "item first.txt",
        "item second.txt",
    ] {
        assert!(stdout.contains(&expected), "{expected:?} in {stdout:?}");
    }
    // A line that is not UTF-8: Windows reads it in the console's OEM code page, Linux
    // replaces the invalid byte.
    let bad = stdout.iter().find(|line| line.starts_with("bad ")).unwrap();
    if cfg!(windows) {
        assert!(
            !bad.contains('\u{fffd}') && bad.ends_with(" byte"),
            "{bad:?}"
        );
    } else {
        assert_eq!(*bad, "bad \u{fffd} byte");
    }
    assert!(
        !stdout
            .iter()
            .any(|line| line.contains('\x1b') || line.contains('\r'))
    );
    let first_exit = lines
        .iter()
        .position(|line| line.text.starts_with("Exit code 4"))
        .unwrap();
    let second_item = lines
        .iter()
        .position(|line| line.text == "item second.txt")
        .unwrap();
    assert!(first_exit < second_item, "the runs are sequential");
    assert_eq!(
        texts(&lines, OutputStream::Stderr),
        ["oops: something", "oops: something"]
    );
    assert_eq!(view.runs.len(), 2);
    assert!(
        view.runs
            .iter()
            .all(|run| run.exit == Some(ProgramExit::Code(4)) && !run.stopped)
    );
    assert!(view.runs[0].command_line.contains("first.txt"));
    assert!(view.finished.is_some_and(|finish| !finish.cancelled));
    drop(running.send);
    tokio::time::timeout(DEADLINE, running.engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_ends_a_running_program_and_starts_no_further_item() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["first.txt", "second.txt"] {
        fs::write(directory.path().join(name), name).unwrap();
    }
    let mut running = start();
    let tool = command("sleep", UserCommandLaunch::PerItem, &["${file.name}"]);
    running
        .send
        .send(Command::ExecuteUserCommand(Box::new(request(
            1,
            tool,
            directory.path(),
            &["first.txt", "second.txt"],
        ))))
        .await
        .unwrap();
    let session = opened(&mut running).await;
    // Stop once the program is running and has printed.
    let until = Instant::now() + DEADLINE;
    while !session.text().contains("sleeping") {
        assert!(Instant::now() < until, "the program printed nothing");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let stopped = Instant::now();
    session.request_stop();
    let (_, view, summary) = finished(&mut running, &session).await;
    assert!(stopped.elapsed() < Duration::from_secs(20));
    assert_eq!((summary.planned, summary.started), (2, 1));
    assert!(summary.cancelled);
    assert_eq!(view.runs.len(), 1);
    assert!(view.runs[0].stopped);
    assert!(view.finished.is_some_and(|finish| finish.cancelled));
    drop(running.send);
    tokio::time::timeout(DEADLINE, running.engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_as_writes_the_text_and_replaces_a_file_only_with_consent() {
    let directory = tempfile::tempdir().unwrap();
    let mut running = start();
    let session = UserCommandOutputSession::new(
        UserCommandRequestId(9),
        UserCommandId::new("tool").unwrap(),
        "Tool".into(),
        1,
    );
    session.append(
        OutputStream::Stdout,
        vec!["first line".into(), "second 文档".into()],
        Some("partial".into()),
    );
    let destination = directory.path().join("output 文档.txt");
    for (request, overwrite, saved) in [(1, false, true), (2, false, false), (3, true, true)] {
        running
            .send
            .send(Command::SaveUserCommandOutput(Box::new(
                UserCommandOutputSaveRequest {
                    request: DesktopRequestId(request),
                    session: session.clone(),
                    destination: destination.clone(),
                    overwrite,
                },
            )))
            .await
            .unwrap();
        let result = tokio::time::timeout(DEADLINE, async {
            loop {
                if let Event::UserCommandOutputSaved {
                    request: answered,
                    result,
                } = running.events.recv().await.unwrap()
                {
                    assert_eq!(answered, DesktopRequestId(request));
                    return result;
                }
            }
        })
        .await
        .unwrap();
        match result {
            Ok(receipt) => {
                assert!(saved, "save {request} must not overwrite: {receipt:?}");
                assert_eq!(receipt.path, destination);
                assert_eq!(receipt.bytes, session.text().len() as u64);
            }
            Err(error) => {
                assert!(!saved, "save {request} failed: {error}");
                assert_eq!(error.kind, std::io::ErrorKind::AlreadyExists);
            }
        }
        // Later output is in the next save.
        session.append(OutputStream::Stdout, vec![format!("after {request}")], None);
    }
    let text = fs::read_to_string(&destination).unwrap();
    assert!(
        text.starts_with("first line\nsecond 文档\nafter 1\nafter 2\n"),
        "{text:?}"
    );
    drop(running.send);
    tokio::time::timeout(DEADLINE, running.engine)
        .await
        .unwrap()
        .unwrap();
}
