#![cfg(windows)]

use midnight_core::{ShellFlavor, quote_shell_path};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Read},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SENTINEL: &str = "__MIDNIGHT_SENTINEL_9A81__";
const ARGV_MARKER: &str = "__MIDNIGHT_NATIVE_ARGV_BEGIN__";
const CHILD_MODE: &str = "MIDNIGHT_CORE_NATIVE_ARGV_CHILD";
const CHILD_OUTPUT: &str = "MIDNIGHT_CORE_NATIVE_ARGV_OUTPUT";

#[test]
fn shell_insertion_rejects_unpaired_utf16_instead_of_retargeting() {
    let path = PathBuf::from(OsString::from_wide(&[0x0043, 0x003a, 0x005c, 0xd800]));
    assert!(quote_shell_path(&path, ShellFlavor::PowerShell).is_err());
}

fn system_program(relative: &str) -> PathBuf {
    let root =
        PathBuf::from(std::env::var_os("SystemRoot").expect("Windows SystemRoot is required"));
    assert!(root.is_absolute());
    root.join("System32").join(relative)
}

const MAX_CAPTURE_BYTES: u64 = 1024 * 1024;

fn read_capture(mut stream: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream
        .by_ref()
        .take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CAPTURE_BYTES {
        // Keep draining so the child cannot stall on a full pipe before the
        // supervising thread observes exit or applies its unchanged deadline.
        io::copy(&mut stream, &mut io::sink())?;
        return Err(io::Error::other(
            "shell output exceeded its 1 MiB capture limit",
        ));
    }
    Ok(bytes)
}

fn run_hidden(mut command: Command) -> Output {
    let mut child = command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the actual system shell");
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // Waiting before reading can deadlock as soon as either pipe fills. Each
    // trusted fixture stream is drained concurrently with bounded capture.
    thread::scope(|scope| {
        let stdout = scope.spawn(move || read_capture(stdout));
        let stderr = scope.spawn(move || read_capture(stderr));
        let deadline = Instant::now() + Duration::from_secs(15);
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break (status, false);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                break (child.wait().unwrap(), true);
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = stdout.join().unwrap().expect("capture shell stdout");
        let stderr = stderr.join().unwrap().expect("capture shell stderr");
        assert!(
            !timed_out,
            "system shell timed out after 15 s; stdout ({} bytes): {}; stderr ({} bytes): {}",
            stdout.len(),
            String::from_utf8_lossy(&stdout[..stdout.len().min(4096)]),
            stderr.len(),
            String::from_utf8_lossy(&stderr[..stderr.len().min(4096)]),
        );
        Output {
            status,
            stdout,
            stderr,
        }
    })
}

#[test]
fn oversized_shell_capture_is_rejected_after_draining_the_stream() {
    let length = MAX_CAPTURE_BYTES as usize + 128 * 1024;
    let mut stream = io::Cursor::new(vec![b'x'; length]);
    let error = read_capture(&mut stream).unwrap_err();
    assert!(error.to_string().contains("capture limit"));
    assert_eq!(stream.position(), length as u64);
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn encoded_command(script: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(ALPHABET[((value >> 18) & 63) as usize] as char);
        encoded.push(ALPHABET[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

#[test]
fn hidden_shell_drains_both_output_streams_before_waiting_for_exit() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "shell_output_child", "--ignored", "--nocapture"])
        .env("MIDNIGHT_CORE_OUTPUT_CHILD", "1");
    let output = run_hidden(command);
    assert!(output.status.success());
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'x').count(),
        128 * 1024
    );
    assert_eq!(output.stderr, vec![b'y'; 128 * 1024]);
}

#[test]
#[ignore = "bounded output subprocess fixture invoked by hidden_shell_drains_both_output_streams_before_waiting_for_exit"]
fn shell_output_child() {
    use std::io::Write;
    if std::env::var_os("MIDNIGHT_CORE_OUTPUT_CHILD").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    std::io::stdout()
        .write_all(&vec![b'x'; 128 * 1024])
        .unwrap();
    std::io::stderr()
        .write_all(&vec![b'y'; 128 * 1024])
        .unwrap();
}
#[test]
fn powershell_argument_context_preserves_unicode_numeric_names_and_inert_shell_syntax() {
    let cases = [
        "",
        "plain",
        "001",
        "0x10",
        "1e3",
        ".001",
        "-literal",
        "\u{2013}literal",
        "\u{2014}literal",
        "\u{2015}literal",
        "two words",
        " \tleading and trailing\t ",
        "line\r\nbreak",
        "no\u{a0}break\u{202f}space",
        "Žluťoučký_中文_日本語_العربية_עברית_👩🏽‍💻_e\u{301}",
        "\u{2018}",
        "\u{2019}",
        "\u{201a}",
        "\u{201b}",
        "\u{201c}",
        "\u{201d}",
        "\u{201e}",
        "\u{201f}",
        "name‘’‚‛“”„‟tail",
        "$HOME",
        "$(Write-Output INJECTED)",
        "back`tick",
        "a; Write-Output INJECTED",
        "literal, @{} [] * ? #",
        "a\"b'c",
        "C:\\plain\\tail\\",
        "C:\\with space\\tail\\",
        "back\\slash\"quote\\",
    ];
    let mut script = String::from(
        "function Read-MidnightArgs { [Console]::Out.WriteLine($args.Count); foreach ($value in $args) { [Console]::Out.WriteLine([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes([string]$value)).Replace('-', '')) } };",
    );
    let mut expected = Vec::new();
    for (index, value) in cases.into_iter().enumerate() {
        let token = quote_shell_path(Path::new(value), ShellFlavor::PowerShell).unwrap();
        // Keep a separate native function invocation and sentinel per case. Only
        // process startup is shared; the numbered markers detect missing output.
        let marker = format!("CASE_{index}");
        script.push_str(&format!(
            "[Console]::Out.WriteLine('{marker}'); Read-MidnightArgs {token} {SENTINEL};"
        ));
        expected.extend([
            marker,
            "2".to_owned(),
            hex(value.as_bytes()),
            hex(SENTINEL.as_bytes()),
        ]);
    }
    let mut command = Command::new(system_program("WindowsPowerShell/v1.0/powershell.exe"));
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
        ])
        .arg(encoded_command(&script));
    let output = run_hidden(command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: Vec<_> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(actual, expected);
}

struct ProbeFile(PathBuf);
impl ProbeFile {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "midnight-core-argv-{}-{id}.txt",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create private argv fixture output: {error}"),
            }
        }
    }
}
impl Drop for ProbeFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
#[ignore = "native argv subprocess fixture invoked by cmd_roundtrip_keeps_each_argument_and_sentinel"]
fn cmd_native_argument_child() {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let values: Vec<_> = std::env::args_os()
        .skip_while(|arg| arg != ARGV_MARKER)
        .skip(1)
        .collect();
    let mut record = format!("{}\n", values.len());
    for value in values {
        let bytes: Vec<_> = value.encode_wide().flat_map(u16::to_le_bytes).collect();
        record.push_str(&hex(&bytes));
        record.push('\n');
    }
    fs::write(std::env::var_os(CHILD_OUTPUT).unwrap(), record).unwrap();
}

#[test]
fn cmd_roundtrip_keeps_each_argument_and_sentinel() {
    let cases = [
        "",
        "plain",
        "001",
        "0x10",
        "1e3",
        "two words",
        " \ttabs\t ",
        "Žluťoučký_中文_日本語_العربية_עברית_👩🏽‍💻_e\u{301}",
        "no\u{a0}break\u{202f}space",
        "‘’‚‛“”„‟",
        "-literal",
        "literal & | ( ) ^",
        "a&echo INJECTED",
        "bang!MIDNIGHT_VALUE!name",
        "$() `ticks`",
        "C:\\plain\\tail\\",
        "C:\\with space\\leaf",
        "embedded\\slash",
    ];
    for value in cases {
        let token = quote_shell_path(Path::new(value), ShellFlavor::Cmd).unwrap();
        let file = ProbeFile::new();
        let mut command = Command::new(system_program("cmd.exe"));
        // /S removes these outer quotes; raw_arg preserves cmd's own syntax.
        // Expanding the trusted executable once also supports '%' in the repo path.
        let tail = format!(
            "\"\"%MIDNIGHT_CORE_ARGV_EXE%\" --exact cmd_native_argument_child --ignored --nocapture -- {ARGV_MARKER} {token} {SENTINEL}\""
        );
        command
            .args(["/D", "/V:OFF", "/S", "/C"])
            .raw_arg(tail)
            .env("MIDNIGHT_CORE_ARGV_EXE", std::env::current_exe().unwrap())
            .env(CHILD_MODE, "1")
            .env(CHILD_OUTPUT, &file.0);
        let output = run_hidden(command);
        assert!(
            output.status.success(),
            "{value:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual: Vec<_> = fs::read_to_string(&file.0)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        let encode = |value: &str| {
            hex(&value
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>())
        };
        assert_eq!(
            actual,
            ["2".to_owned(), encode(value), encode(SENTINEL)],
            "{value:?}: {token}"
        );
    }
}

#[test]
fn cmd_refuses_expansion_newlines_quotes_nul_and_ambiguous_quoted_trailing_slashes() {
    for value in [
        "%PATH%",
        "a%X%b",
        "a\"b",
        "a\rb",
        "a\nb",
        "a\0b",
        "C:\\with space\\",
        "tail space\\",
    ] {
        assert!(
            quote_shell_path(Path::new(value), ShellFlavor::Cmd).is_err(),
            "{value:?}"
        );
    }
}
