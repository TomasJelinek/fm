//! Run directly with output: a rerun of this test binary is the program, reading its
//! own arguments after the first `--`; the literal argv, which holds bytes that are not
//! UTF-8, goes to [`echo_program`] instead.
use super::*;
use std::{
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const PROGRAM: &str = "user_command_output::tests::program_fixture";
/// Elapses only on failure.
const DEADLINE: Duration = Duration::from_secs(60);
/// What follows libtest's own banner in the fixture's standard output.
const MARKER: &str = "<<<fixture>>>";

fn rerun(mode: &[&str], extra: &[OsString]) -> Vec<OsString> {
    [
        "--exact",
        PROGRAM,
        "--ignored",
        "--nocapture",
        "--test-threads=1",
        "--",
    ]
    .iter()
    .chain(mode)
    .map(OsString::from)
    .chain(extra.iter().cloned())
    .collect()
}

fn own_arguments() -> Vec<OsString> {
    let all: Vec<OsString> = std::env::args_os().collect();
    let at = all.iter().position(|argument| argument == "--").unwrap();
    all[at + 1..].to_vec()
}

fn literal_arguments() -> Vec<OsString> {
    let mut arguments: Vec<OsString> = [
        "clean",
        "-fxd",
        "",
        "two words",
        "say \"hi\"",
        "it's",
        r"back\slash\",
        "e\u{301} 文档 👩🏽‍💻",
        "$HOME `id` $(id) ; | & *",
        " ",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    // Bytes that are not UTF-8 stay as they are.
    arguments.push(OsString::from_vec(b"raw \xff\xfe".to_vec()));
    arguments
}

/// The program that echoes its literal argv: `/bin/sh` writing each argument and a line
/// break to `argv.txt` and to standard output. Not a rerun of this test binary: libtest
/// reads its own command line with `std::env::args` and panics on an argument that is not
/// UTF-8 before any test runs.
fn echo_program(arguments: &[OsString]) -> Vec<OsString> {
    [
        "/bin/sh",
        "-c",
        r#"printf '%s\n' "$@" > argv.txt && cat argv.txt"#,
        "sh",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(arguments.iter().cloned())
    .collect()
}

/// What [`echo_program`] writes for `arguments` (none holds a line break).
fn listed(arguments: &[OsString]) -> Vec<u8> {
    arguments
        .iter()
        .flat_map(|argument| argument.as_bytes().iter().chain(b"\n"))
        .copied()
        .collect()
}

fn command(mode: &[&str], extra: &[OsString], directory: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(rerun(mode, extra)).current_dir(directory);
    command
}

#[test]
#[ignore = "program run by the captured output tests"]
fn program_fixture() {
    let arguments = own_arguments();
    let mode = arguments[0].to_str().unwrap().to_owned();
    let rest = &arguments[1..];
    let mut out = io::stdout().lock();
    let mut err = io::stderr().lock();
    writeln!(out, "{MARKER}").unwrap();
    let mut code = 0;
    match mode.as_str() {
        "interleave" => {
            for index in 0..200 {
                writeln!(out, "out {index}").unwrap();
                out.flush().unwrap();
                writeln!(err, "err {index}").unwrap();
                err.flush().unwrap();
            }
        }
        "burst" => {
            let line = [b'x'; 1023]
                .iter()
                .chain(b"\n")
                .copied()
                .collect::<Vec<_>>();
            for _ in 0..32 * 1024 {
                out.write_all(&line).unwrap();
            }
            for _ in 0..4 * 1024 {
                err.write_all(&line).unwrap();
            }
        }
        "stdin" => {
            let mut input = Vec::new();
            let read = io::stdin().read_to_end(&mut input);
            writeln!(out, "stdin {} {}", read.is_ok(), input.len()).unwrap();
        }
        "tree" => {
            let mut grandchild = Command::new(std::env::current_exe().unwrap())
                .args(rerun(&["sleep"], &[]))
                .spawn()
                .unwrap();
            fs::write("grandchild.txt", grandchild.id().to_string()).unwrap();
            // Until Stop ends both (the grandchild sleeps that long).
            let _ = grandchild.wait();
        }
        "sleep" => thread::sleep(DEADLINE),
        "exit" => code = rest[0].to_str().unwrap().parse().unwrap(),
        other => panic!("unknown fixture mode {other}"),
    }
    out.flush().unwrap();
    err.flush().unwrap();
    std::process::exit(code);
}

struct Collected {
    exit: ProgramExit,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn collect(mut program: Box<dyn CapturedUserProgram>) -> Collected {
    let streams = program.take_streams().unwrap();
    assert!(program.take_streams().is_none());
    let read = |mut pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes).unwrap();
            bytes
        })
    };
    let stdout = read(streams.stdout);
    let stderr = read(streams.stderr);
    let exit = wait(program.as_mut());
    Collected {
        exit,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

fn wait(program: &mut dyn CapturedUserProgram) -> ProgramExit {
    let until = Instant::now() + DEADLINE;
    loop {
        if let Some(exit) = program.try_wait().unwrap() {
            return exit;
        }
        assert!(Instant::now() < until, "the program did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

fn after_marker(bytes: &[u8]) -> Vec<u8> {
    let marker = format!("{MARKER}\n");
    let at = bytes
        .windows(marker.len())
        .position(|window| window == marker.as_bytes())
        .expect("the fixture's marker");
    bytes[at + marker.len()..].to_vec()
}

#[test]
fn captured_programs_receive_the_literal_argv_and_an_empty_closed_stdin() {
    let directory = tempfile::tempdir().unwrap();
    let arguments = literal_arguments();
    let program = echo_program(&arguments);
    let mut echo = Command::new(&program[0]);
    echo.args(&program[1..]).current_dir(directory.path());
    let collected = collect(spawn_captured(echo).unwrap());
    assert_eq!(collected.exit, ProgramExit::Code(0));
    assert_eq!(
        fs::read(directory.path().join("argv.txt")).unwrap(),
        listed(&arguments)
    );
    assert_eq!(collected.stdout, listed(&arguments));
    let collected = collect(spawn_captured(command(&["stdin"], &[], directory.path())).unwrap());
    assert_eq!(after_marker(&collected.stdout), b"stdin true 0\n");
}

#[test]
fn exit_codes_signals_and_interleaved_streams() {
    let directory = tempfile::tempdir().unwrap();
    for code in [0, 3, 255] {
        let collected = collect(
            spawn_captured(command(&["exit", &code.to_string()], &[], directory.path())).unwrap(),
        );
        assert_eq!(collected.exit, ProgramExit::Code(code));
    }
    let collected =
        collect(spawn_captured(command(&["interleave"], &[], directory.path())).unwrap());
    let stdout = String::from_utf8(after_marker(&collected.stdout)).unwrap();
    let stderr = String::from_utf8(collected.stderr).unwrap();
    assert!(
        stdout
            .lines()
            .enumerate()
            .all(|(i, line)| line == format!("out {i}"))
    );
    assert!(
        stderr
            .lines()
            .enumerate()
            .all(|(i, line)| line == format!("err {i}"))
    );
    assert_eq!(stdout.lines().count(), 200);
    assert_eq!(stderr.lines().count(), 200);
    let mut sleeping = spawn_captured(command(&["sleep"], &[], directory.path())).unwrap();
    sleeping.terminate_tree(true).unwrap();
    assert_eq!(wait(sleeping.as_mut()), ProgramExit::Signal(libc::SIGKILL));
}

#[test]
fn a_large_burst_on_both_streams_never_deadlocks() {
    let directory = tempfile::tempdir().unwrap();
    let collected = collect(spawn_captured(command(&["burst"], &[], directory.path())).unwrap());
    assert_eq!(collected.exit, ProgramExit::Code(0));
    assert_eq!(after_marker(&collected.stdout).len(), 32 * 1024 * 1024);
    assert_eq!(collected.stderr.len(), 4 * 1024 * 1024);
}

/// Running or not yet reaped: a zombie has ended.
fn running(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z" && state != "X")
    })
}

#[test]
fn stop_ends_the_program_and_its_process_group() {
    let directory = tempfile::tempdir().unwrap();
    let mut program = spawn_captured(command(&["tree"], &[], directory.path())).unwrap();
    let streams = program.take_streams().unwrap();
    let (closed, pipe_closed) = mpsc::channel();
    for mut pipe in [streams.stdout, streams.stderr] {
        let closed = closed.clone();
        thread::spawn(move || {
            let _ = io::copy(&mut pipe, &mut io::sink());
            let _ = closed.send(());
        });
    }
    let marker = directory.path().join("grandchild.txt");
    let until = Instant::now() + DEADLINE;
    let grandchild = loop {
        if let Ok(id) = fs::read_to_string(&marker)
            && let Ok(id) = id.parse::<u32>()
        {
            break id;
        }
        assert!(Instant::now() < until, "the grandchild did not start");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(running(grandchild));
    program.terminate_tree(false).unwrap();
    assert_eq!(wait(program.as_mut()), ProgramExit::Signal(libc::SIGTERM));
    let until = Instant::now() + Duration::from_secs(10);
    while running(grandchild) {
        assert!(Instant::now() < until, "the grandchild outlived Stop");
        thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..2 {
        pipe_closed.recv_timeout(DEADLINE).expect("the pipes close");
    }
}

#[test]
fn console_window_runs_the_launcher_with_the_literal_argv_in_the_terminal() {
    let launcher = ConsoleLauncher {
        executable: PathBuf::from("/opt/midnight/me"),
        leading: vec![midnight_core::RUN_AND_WAIT_ARGUMENT.into()],
    };
    let arguments = literal_arguments();
    let program = Path::new("/usr/bin/git");
    for (terminal, options) in [
        ("/usr/bin/xterm", &["-e"][..]),
        ("/usr/bin/gnome-terminal", &["--wait", "--"][..]),
        ("/usr/bin/konsole", &["--nofork", "-e"][..]),
    ] {
        let command = console_command_with(
            Path::new(terminal),
            &launcher,
            program,
            &arguments,
            Path::new("/tmp"),
        )
        .unwrap();
        assert_eq!(command.get_program(), terminal);
        let mut expected: Vec<OsString> = options.iter().map(OsString::from).collect();
        expected.push("/opt/midnight/me".into());
        expected.push("--run-and-wait".into());
        expected.push("/usr/bin/git".into());
        expected.extend(arguments.iter().cloned());
        assert_eq!(
            command.get_args().map(OsString::from).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/tmp")));
    }
}

#[test]
fn the_launcher_runs_the_literal_argv_and_reports_how_it_ended() {
    let directory = tempfile::tempdir().unwrap();
    let arguments = literal_arguments();
    let program = echo_program(&arguments);
    let mut report = Vec::new();
    let code = run_program(program.clone(), Some(directory.path()), &mut report, false);
    assert_eq!(code, 0);
    assert_eq!(
        fs::read(directory.path().join("argv.txt")).unwrap(),
        listed(&arguments)
    );
    let report = String::from_utf8(report).unwrap();
    assert!(
        report.starts_with(&format!(
            "> {}\n",
            midnight_core::display_command_line(Path::new(&program[0]), &program[1..])
        )),
        "{report}"
    );
    assert!(report.contains("\nExit code 0 · "), "{report}");
    let mut report = Vec::new();
    assert_eq!(
        run_program(
            vec!["/nonexistent/program".into()],
            None,
            &mut report,
            false
        ),
        1
    );
    assert!(
        String::from_utf8(report)
            .unwrap()
            .contains("could not start")
    );
}
