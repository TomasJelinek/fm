//! Owned native-process test helper. Reports runtime argv/cwd/environment as raw records.
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::PathBuf,
    process::{Command, Stdio},
};
#[cfg(windows)]
#[path = "windows.rs"]
mod native;
#[cfg(unix)]
#[path = "unix.rs"]
mod native;

fn record(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    output.extend_from_slice(bytes);
}
fn main() -> io::Result<()> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    // Test terminal fixture only: execute the real selected shell through its normal argv.
    if arguments.first().is_some_and(|value| value == "-e") {
        let shell = arguments
            .get(1)
            .ok_or_else(|| io::Error::other("missing fixture shell"))?;
        let status = Command::new(shell)
            .args(&arguments[2..])
            .stdin(Stdio::null())
            .status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
    let destination = PathBuf::from(
        arguments
            .first()
            .ok_or_else(|| io::Error::other("missing owned output path"))?,
    );
    let mut output = b"MECMD1\0".to_vec();
    record(&mut output, &native::bytes(env::current_dir()?.as_os_str()));
    output.extend_from_slice(&((arguments.len() - 1) as u32).to_le_bytes());
    for argument in &arguments[1..] {
        record(&mut output, &native::bytes(argument));
    }
    let environment = [
        "ME_ACTIVE_PATH",
        "ME_OTHER_PATH",
        "ME_FILE_PATH",
        "ME_FILE_NAME",
        "ME_FILE_STEM",
        "ME_FILE_EXTENSION",
    ]
    .into_iter()
    .filter_map(|name| env::var_os(name).map(|value| (OsString::from(name), value)))
    .collect::<Vec<_>>();
    output.extend_from_slice(&(environment.len() as u32).to_le_bytes());
    for (name, value) in environment {
        record(&mut output, &native::bytes(&name));
        record(&mut output, &native::bytes(&value));
    }
    fs::write(destination, output)
}
