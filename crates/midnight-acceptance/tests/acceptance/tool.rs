//! Harmless programs for the scenarios that configure external editors, comparators and
//! Custom Commands: `cp`, `touch`, `true` and `sh` running the suite's one literal
//! script ([`LIST_SCRIPT`]). On Unix they are the system's own. Windows has none of
//! them, so there [`program`] gives a stand-in: this test executable under that name
//! (`cp.exe`, …, hard-linked or copied into the run's `work/.tools/`), which acts as
//! the program its file name names when the application starts it:
//!
//! - `cp FILE DEST`: copy FILE to DEST, or into DEST when it is a folder;
//! - `touch PATH`: create PATH (relative to the working folder) unless it exists;
//! - `true`: do nothing and succeed;
//! - `sh -c LIST_SCRIPT NAME ARG...`: write each ARG on its own line to
//!   `selection.txt` in the working folder, as the script does; `sh -c true` succeeds;
//!   any other script is refused.
//!
//! Each writes only where its arguments point, which the scenarios keep inside their own
//! work folders; nothing else runs, so no scenario starts a terminal or a real editor.
use super::Context;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

/// The grouped command's script: one line per argument after `$0`, written into its
/// working folder (`sh -c LIST_SCRIPT listing ARG...`).
pub const LIST_SCRIPT: &str = "for f in \"$@\"; do printf '%s\\n' \"$f\"; done > selection.txt";

/// The programs [`program`] knows.
const NAMES: [&str; 4] = ["cp", "touch", "true", "sh"];

/// The harmless program `name` (`cp`, `touch`, `true` or `sh`): the system's on Unix, a
/// stand-in in the run's `work/.tools/` folder on Windows.
pub fn program(context: &Context, name: &str) -> Result<PathBuf, String> {
    if !NAMES.contains(&name) {
        return Err(format!(
            "{name} is not one of the suite's harmless programs"
        ));
    }
    #[cfg(unix)]
    {
        let _ = context;
        let candidates = [format!("/bin/{name}"), format!("/usr/bin/{name}")];
        candidates
            .iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .ok_or_else(|| format!("none of {candidates:?} exists on this machine"))
    }
    #[cfg(not(unix))]
    {
        let folder = context.run_dir.join("work").join(".tools");
        let path = folder.join(format!("{name}.exe"));
        if path.is_file() {
            return Ok(path);
        }
        fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        let this = std::env::current_exe()
            .map_err(|error| format!("cannot locate this test executable: {error}"))?;
        if fs::hard_link(&this, &path).is_err() {
            fs::copy(&this, &path).map_err(|error| {
                format!("cannot place the stand-in {}: {error}", path.display())
            })?;
        }
        Ok(path)
    }
}

/// Act as the harmless program this executable's file name names, if it does.
pub fn run_if_requested() -> Option<ExitCode> {
    let executable = std::env::current_exe().ok()?;
    let name = executable.file_stem()?.to_str()?.to_owned();
    if !NAMES.contains(&name.as_str()) {
        return None;
    }
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let result = match (name.as_str(), arguments.as_slice()) {
        ("cp", [file, destination]) => copy(file, destination),
        ("touch", [path]) => fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(drop)
            .map_err(|error| format!("touch {}: {error}", path.display())),
        ("true", _) => Ok(()),
        ("sh", [flag, script, _, items @ ..])
            if flag.as_os_str() == "-c" && script.as_os_str() == LIST_SCRIPT =>
        {
            list_into(Path::new("selection.txt"), items)
        }
        ("sh", [flag, script, ..]) if flag.as_os_str() == "-c" && script.as_os_str() == "true" => {
            Ok(())
        }
        _ => Err(format!(
            "this stand-in {name} supports only: cp FILE DEST, touch PATH, true, sh -c \
             <the suite's list script> NAME ARG..."
        )),
    };
    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    })
}

fn copy(file: &Path, destination: &Path) -> Result<(), String> {
    let target = if destination.is_dir() {
        destination.join(
            file.file_name()
                .ok_or_else(|| format!("{} has no file name", file.display()))?,
        )
    } else {
        destination.to_path_buf()
    };
    fs::copy(file, &target)
        .map(drop)
        .map_err(|error| format!("cp {} {}: {error}", file.display(), target.display()))
}

fn list_into(path: &Path, items: &[PathBuf]) -> Result<(), String> {
    let mut file =
        fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?;
    for item in items {
        writeln!(file, "{}", item.display())
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}
