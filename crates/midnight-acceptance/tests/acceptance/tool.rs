//! Harmless programs for the scenarios that configure external editors, comparators and
//! Custom Commands: `cp`, `touch`, `true` and `sh` running the suite's one literal
//! script ([`LIST_SCRIPT`]). On Unix they are the system's own. Windows has none of
//! them, so there [`program`] gives a stand-in: this test executable under that name
//! (`cp.exe`, …, hard-linked beside this executable in its private installation or
//! stage, else hard-linked or copied into the run's `work/.tools/`), which acts as the
//! program its file name names when the application starts it:
//!
//! - `cp FILE DEST`: copy FILE to DEST, or into DEST when it is a folder;
//! - `touch PATH`: create PATH (relative to the working folder) unless it exists;
//! - `true`: do nothing and succeed;
//! - `sh -c LIST_SCRIPT NAME ARG...`: write each ARG on its own line to
//!   `selection.txt` in the working folder, as the script does; `sh -c true` succeeds;
//! - `sh -c OUTPUT_SCRIPT NAME ARG...`: print `out ARG` for each ARG, `err line` on
//!   standard error and exit with status 3, as the script does; any other script is
//!   refused.
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

/// The output window's script: `out ARG` per argument after `$0` on standard output, one
/// line on standard error, exit status 3 (`sh -c OUTPUT_SCRIPT output ARG...`).
pub const OUTPUT_SCRIPT: &str =
    "for f in \"$@\"; do printf 'out %s\\n' \"$f\"; done; printf 'err line\\n' >&2; exit 3";

/// The programs [`program`] knows.
const NAMES: [&str; 4] = ["cp", "touch", "true", "sh"];

/// The harmless program `name` (`cp`, `touch`, `true` or `sh`): the system's on Unix, a
/// stand-in on Windows (see [`stand_in`]).
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
        let this = std::env::current_exe()
            .map_err(|error| format!("cannot locate this test executable: {error}"))?;
        let path = stand_in(context, &this, name)?;
        warm(&path);
        Ok(path)
    }
}

/// Start a stand-in once, without arguments (it refuses them and exits), before a
/// scenario times one of its starts: the first start of a new link to this executable of
/// over 100 MB can take seconds (in the Windows run of 0aba1c1a the first `cp` of
/// `find_result_actions` wrote its copy only after about 15 s), which is no part of what
/// a scenario checks. Once per path and process.
#[cfg(not(unix))]
fn warm(path: &Path) {
    use std::process::{Command, Stdio};
    static WARMED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut warmed) = WARMED.lock() else {
        return;
    };
    if warmed.iter().any(|done| done == path) {
        return;
    }
    let _ = Command::new(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    warmed.push(path.to_path_buf());
}

/// This executable `this` as `<name>.exe`. The test executable is over 100 MB, and a
/// hard link needs the volume it is on: `scripts/acceptance.ps1` runs it from a private
/// installation below the user profile (`MIDNIGHT_ACCEPTANCE_STAGE_ROOT`), usually on
/// another drive than the run folder, and a staged child runs a link to it in a stage
/// there. Run from such a place, the stand-ins are hard links in a `.tools-<run>` folder
/// beside it, which goes with the installation or the stage. Otherwise, or when that
/// link fails, they go into the run's `work\.tools\`: hard links when the run folder
/// shares the executable's volume (the default `_build\acceptance` beside a Cargo
/// build), else copies.
#[cfg(not(unix))]
fn stand_in(context: &Context, this: &Path, name: &str) -> Result<PathBuf, String> {
    let file = format!("{name}.exe");
    let private = std::env::var_os("MIDNIGHT_ACCEPTANCE_STAGE_ROOT")
        .is_some_and(|root| Path::new(&root).is_absolute() && this.starts_with(&root));
    if private && let (Some(beside), Some(run)) = (this.parent(), context.run_dir.file_name()) {
        let mut tools = std::ffi::OsString::from(".tools-");
        tools.push(run);
        let folder = beside.join(tools);
        let path = folder.join(&file);
        let linked = || fs::create_dir_all(&folder).is_ok() && fs::hard_link(this, &path).is_ok();
        if path.is_file() || linked() {
            return Ok(path);
        }
    }
    let folder = context.run_dir.join("work").join(".tools");
    let path = folder.join(&file);
    if path.is_file() {
        return Ok(path);
    }
    fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    if fs::hard_link(this, &path).is_err() {
        fs::copy(this, &path)
            .map_err(|error| format!("cannot place the stand-in {}: {error}", path.display()))?;
    }
    Ok(path)
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
        ("sh", [flag, script, _, items @ ..])
            if flag.as_os_str() == "-c" && script.as_os_str() == OUTPUT_SCRIPT =>
        {
            for item in items {
                println!("out {}", item.display());
            }
            eprintln!("err line");
            return Some(ExitCode::from(3));
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
