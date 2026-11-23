//! Run directly with output: a rerun of this test binary is the program (and the
//! console launcher), reading its own arguments after the first `--`.
use super::*;
use midnight_core::{Cancellation, UserCommandService};
use std::{
    fs,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{FALSE, WAIT_OBJECT_0},
    Storage::FileSystem::{FILE_TYPE_CHAR, FILE_TYPE_PIPE, GetFileType},
    System::{
        Console::{GetStdHandle, STD_OUTPUT_HANDLE},
        Threading::{OpenProcess, PROCESS_SYNCHRONIZE},
    },
};

const PROGRAM: &str = "user_command_output::tests::program_fixture";
const LAUNCHER: &str = "user_command_output::tests::launcher_fixture";
/// Elapses only on failure.
const DEADLINE: Duration = Duration::from_secs(60);
/// What follows libtest's own banner in the fixture's standard output.
const MARKER: &str = "<<<fixture>>>";

/// Arguments that make this test binary run `fixture` with `mode` and `extra`.
fn rerun(fixture: &str, mode: &[&str], extra: &[OsString]) -> Vec<OsString> {
    [
        "--exact",
        fixture,
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

/// A fixture's own arguments: those after the first `--`.
fn own_arguments() -> Vec<OsString> {
    let all: Vec<OsString> = std::env::args_os().collect();
    let at = all.iter().position(|argument| argument == "--").unwrap();
    all[at + 1..].to_vec()
}

fn literal_arguments() -> Vec<OsString> {
    [
        "clean",
        "-fxd",
        "",
        "two words",
        "say \"hi\"",
        r"trailing\",
        r"C:\Program Files\x\",
        "e\u{301} 文档 👩🏽‍💻",
        r#"\"already\" quoted"#,
        "a\\\\b",
        "%PATH% & | ^ ! ;",
        " ",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

fn listed(arguments: &[OsString]) -> String {
    arguments
        .iter()
        .map(|argument| format!("{argument:?}\n"))
        .collect()
}

#[test]
#[ignore = "program run by the captured and console tests"]
fn program_fixture() {
    let arguments = own_arguments();
    let mode = arguments[0].to_str().unwrap().to_owned();
    let rest = &arguments[1..];
    let mut out = io::stdout().lock();
    let mut err = io::stderr().lock();
    writeln!(out, "{MARKER}").unwrap();
    let mut code = 0;
    match mode.as_str() {
        "echo" => {
            fs::write("argv.txt", listed(rest)).unwrap();
            out.write_all(listed(rest).as_bytes()).unwrap();
        }
        "streams" => {
            // SAFETY: a query on this process's own standard output handle.
            let kind = unsafe { GetFileType(GetStdHandle(STD_OUTPUT_HANDLE)) };
            fs::write("streams.txt", kind.to_string()).unwrap();
        }
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
            // A grandchild that shares the pipes and outlives its parent unless Stop
            // ends the whole tree.
            let mut grandchild = std::process::Command::new(std::env::current_exe().unwrap())
                .args(rerun(PROGRAM, &["sleep"], &[]))
                .spawn()
                .unwrap();
            fs::write("grandchild.txt", grandchild.id().to_string()).unwrap();
            // Until Stop ends both (the grandchild sleeps that long).
            let _ = grandchild.wait();
        }
        "sleep" => thread::sleep(DEADLINE),
        "exit" => code = rest[0].to_str().unwrap().parse::<u32>().unwrap() as i32,
        other => panic!("unknown fixture mode {other}"),
    }
    out.flush().unwrap();
    err.flush().unwrap();
    // Exit before libtest writes its summary into the captured output.
    std::process::exit(code);
}

#[test]
#[ignore = "console launcher run by console_window_passes_the_literal_argv"]
fn launcher_fixture() {
    let mut report = fs::File::create("report.txt").unwrap();
    let code = run_program(own_arguments(), &mut report, false);
    std::process::exit(code);
}

struct Collected {
    exit: ProgramExit,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Read both pipes concurrently until they close and wait for the exit.
fn collect(mut program: Box<dyn CapturedUserProgram>) -> Collected {
    let streams = program.take_streams().unwrap();
    assert!(
        program.take_streams().is_none(),
        "the pipes are handed out once"
    );
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

fn after_marker(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let at = text.find(MARKER).expect("the fixture's marker") + MARKER.len() + 1;
    text[at..].to_owned()
}

fn spawn(mode: &[&str], extra: &[OsString], directory: &Path) -> Box<dyn CapturedUserProgram> {
    spawn_captured(
        &std::env::current_exe().unwrap(),
        &rerun(PROGRAM, mode, extra),
        directory,
    )
    .unwrap()
}

#[test]
fn hidden_programs_write_into_nothing_and_captured_ones_into_pipes() {
    // Hidden (Run directly before this choice existed): standard output is the NUL
    // device, so nothing the program prints can be shown.
    let directory = crate::test_fixtures::build_temp("hidden-");
    let filesystem = crate::WindowsFileSystem::new(None);
    let mut hidden = filesystem
        .launch_user_program(
            &std::env::current_exe().unwrap(),
            &rerun(PROGRAM, &["streams"], &[]),
            directory.path(),
            &Cancellation::default(),
        )
        .unwrap();
    let until = Instant::now() + DEADLINE;
    while hidden.try_wait().unwrap().is_none() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(10));
    }
    let kind = fs::read_to_string(directory.path().join("streams.txt")).unwrap();
    assert_eq!(
        kind,
        FILE_TYPE_CHAR.to_string(),
        "NUL is a character device"
    );
    // Output window: a pipe the app reads.
    let captured = filesystem
        .launch_user_program_captured(
            &std::env::current_exe().unwrap(),
            &rerun(PROGRAM, &["streams"], &[]),
            directory.path(),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(collect(captured).exit, ProgramExit::Code(0));
    let kind = fs::read_to_string(directory.path().join("streams.txt")).unwrap();
    assert_eq!(kind, FILE_TYPE_PIPE.to_string());
}

#[test]
fn captured_programs_receive_the_literal_argv_and_an_empty_closed_stdin() {
    let directory = crate::test_fixtures::build_temp("argv-文档-");
    let arguments = literal_arguments();
    let collected = collect(spawn(&["echo"], &arguments, directory.path()));
    assert_eq!(collected.exit, ProgramExit::Code(0));
    assert_eq!(
        fs::read_to_string(directory.path().join("argv.txt")).unwrap(),
        listed(&arguments)
    );
    assert_eq!(after_marker(&collected.stdout), listed(&arguments));
    // Standard input is the NUL device: reading it ends at once, so nothing waits for
    // input that can never come.
    let started = Instant::now();
    let collected = collect(spawn(&["stdin"], &[], directory.path()));
    assert_eq!(after_marker(&collected.stdout), "stdin true 0\n");
    assert!(started.elapsed() < DEADLINE / 2);
}

#[test]
fn exit_codes_and_interleaved_streams_arrive_whole() {
    let directory = crate::test_fixtures::build_temp("streams-");
    for code in [0u32, 3, 259, 0xC000_013A] {
        let collected = collect(spawn(&["exit", &code.to_string()], &[], directory.path()));
        assert_eq!(collected.exit, ProgramExit::Code(i64::from(code)));
    }
    let collected = collect(spawn(&["interleave"], &[], directory.path()));
    let stdout = after_marker(&collected.stdout);
    let stderr = String::from_utf8(collected.stderr).unwrap();
    assert_eq!(stdout.lines().count(), 200);
    assert_eq!(stderr.lines().count(), 200);
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
}

#[test]
fn a_large_burst_on_both_streams_never_deadlocks() {
    let directory = crate::test_fixtures::build_temp("burst-");
    let collected = collect(spawn(&["burst"], &[], directory.path()));
    assert_eq!(collected.exit, ProgramExit::Code(0));
    assert_eq!(after_marker(&collected.stdout).len(), 32 * 1024 * 1024);
    assert_eq!(collected.stderr.len(), 4 * 1024 * 1024);
}

#[test]
fn stop_ends_the_program_and_every_process_it_started() {
    let directory = crate::test_fixtures::build_temp("tree-");
    let mut program = spawn(&["tree"], &[], directory.path());
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
    // SAFETY: wait-only access to the grandchild; ownership after success.
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, FALSE, grandchild) };
    assert!(!raw.is_null());
    let grandchild = unsafe { OwnedHandle::from_raw_handle(raw) };
    assert_eq!(program.try_wait().unwrap(), None);
    program.terminate_tree(false).unwrap();
    assert_eq!(wait(program.as_mut()), ProgramExit::Code(1));
    // SAFETY: a live owned process handle.
    assert_eq!(
        unsafe { WaitForSingleObject(grandchild.as_raw_handle(), 10_000) },
        WAIT_OBJECT_0,
        "the grandchild outlived Stop"
    );
    for _ in 0..2 {
        pipe_closed
            .recv_timeout(DEADLINE)
            .expect("the pipes close once the tree has ended");
    }
}

#[test]
fn console_window_passes_the_literal_argv_through_the_launcher() {
    let directory = crate::test_fixtures::build_temp("console-文档-");
    let arguments = literal_arguments();
    let executable = std::env::current_exe().unwrap();
    let launcher = ConsoleLauncher {
        executable: executable.clone(),
        leading: rerun(LAUNCHER, &[], &[]),
    };
    let program_arguments = rerun(PROGRAM, &["echo"], &arguments);
    let mut console = spawn_console_with(
        &launcher,
        &executable,
        &program_arguments,
        directory.path(),
        true,
    )
    .unwrap();
    let until = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = console.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < until, "the launcher did not finish");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(0));
    assert_eq!(
        fs::read_to_string(directory.path().join("argv.txt")).unwrap(),
        listed(&arguments)
    );
    let report = fs::read_to_string(directory.path().join("report.txt")).unwrap();
    assert!(
        report.starts_with(&format!(
            "> {}\n",
            midnight_core::display_command_line(&executable, &program_arguments)
        )),
        "{report}"
    );
    assert!(report.contains("\nExit code 0 · "), "{report}");
}

#[test]
fn console_window_needs_the_application_launcher() {
    // Tests never set the process-wide launcher: the application does at startup.
    let error = spawn_console(
        &std::env::current_exe().unwrap(),
        &[],
        &std::env::temp_dir(),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
}

#[test]
fn console_output_decodes_as_utf8_first_then_the_oem_code_page() {
    assert_eq!(decode_console("café 文档".as_bytes()), "café 文档");
    // 0x82 is "é" in the common OEM code pages (437, 850, 852); never a replacement.
    let decoded = decode_console(b"caf\x82");
    assert!(decoded.starts_with("caf"), "{decoded}");
    assert!(!decoded.contains('\u{fffd}'), "{decoded}");
}
