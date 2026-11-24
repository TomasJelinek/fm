//! Installing a staged release: the swap, the new version's startup check and the
//! rollback.
//!
//! "Restart now" ends the running copy; before it exits, it copies its own executable
//! to `.update/updater/` and starts that copy with `--apply-update`. The updater is the
//! previous version's code, which has proved it runs on this machine, so it can restore
//! the previous version whatever the new one does. It:
//!
//! 1. waits for an exclusive lock on `.update/instances.lock`, which every running copy
//!    holds shared, so no copy runs from the folder (60 s, then it gives up);
//! 2. checks the staged release against its `BUILD-INFO.json` again;
//! 3. moves every entry of the installation folder except `.update` into
//!    `.update/backup/`, then every entry of the staged release into the folder. Both
//!    are renames on one volume; a failed step moves everything back;
//! 4. starts the new executable with the original arguments and working folder, and
//!    waits for its startup check: the new version writes `.update/confirmed` once its
//!    window shows with the user's settings and panels restored;
//! 5. on confirmation removes the backup; when the new version exits before confirming,
//!    moves its files to `.update/failed/`, restores the backup and starts the previous
//!    version again; when neither happens in time, keeps the backup and leaves the
//!    update to the new version: the first start after its confirmation removes the
//!    backup (`Pending::unattended`).
//!
//! `result.json` tells the next start how it went; `update.log` records every step.
use crate::{
    archive::{self, BuildInfo},
    install::{self, WORK_FOLDER, lock_file, remove_tree, work_folder, write_json},
    platform,
    release::PLATFORM,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

/// The first argument of an updater process.
pub const APPLY_ARGUMENT: &str = "--apply-update";

/// How long the updater waits for each step.
#[derive(Clone, Copy, Debug)]
pub struct Timings {
    /// For every running copy to exit.
    pub instances: Duration,
    /// For one rename that a scanner or a closing process holds up.
    pub rename: Duration,
    /// For the new version's startup check.
    pub confirm: Duration,
    pub poll: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            instances: Duration::from_secs(60),
            rename: Duration::from_secs(15),
            confirm: Duration::from_secs(180),
            poll: Duration::from_millis(200),
        }
    }
}

/// What the updater is asked to do. Travels as command-line arguments, so paths and
/// arguments keep their native form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyRequest {
    pub installation: PathBuf,
    pub staged: PathBuf,
    pub working_folder: PathBuf,
    pub arguments: Vec<OsString>,
}

impl ApplyRequest {
    /// `--apply-update INSTALLATION STAGED WORKING-FOLDER -- ARGUMENTS...`
    pub fn to_arguments(&self) -> Vec<OsString> {
        let mut arguments = vec![
            OsString::from(APPLY_ARGUMENT),
            self.installation.clone().into_os_string(),
            self.staged.clone().into_os_string(),
            self.working_folder.clone().into_os_string(),
            OsString::from("--"),
        ];
        arguments.extend(self.arguments.iter().cloned());
        arguments
    }

    /// Parses the arguments after the program name; `None` unless the first one is
    /// `--apply-update`.
    pub fn from_arguments(arguments: &[OsString]) -> Option<Result<Self, String>> {
        if arguments
            .first()
            .is_none_or(|first| first != APPLY_ARGUMENT)
        {
            return None;
        }
        let parsed = (|| {
            let installation = PathBuf::from(arguments.get(1)?);
            let staged = PathBuf::from(arguments.get(2)?);
            let working_folder = PathBuf::from(arguments.get(3)?);
            (arguments.get(4)? == "--").then_some(())?;
            Some(Self {
                installation,
                staged,
                working_folder,
                arguments: arguments[5..].to_vec(),
            })
        })();
        Some(parsed.ok_or_else(|| "Invalid updater arguments.".to_owned()))
    }

    fn work(&self) -> PathBuf {
        work_folder(&self.installation)
    }
}

/// How an update went, kept in `.update/result.json` for the next start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Outcome {
    Updated {
        from: String,
        to: String,
    },
    /// The new version did not pass its startup check; the previous one is back.
    RolledBack {
        from: String,
        to: String,
        reason: String,
    },
    /// Nothing changed.
    NotApplied {
        reason: String,
    },
    /// The new version is running but did not confirm in time; the backup is kept.
    Unconfirmed {
        from: String,
        to: String,
    },
}

/// Marks the update in progress; the new version confirms when `to` is its version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub from: String,
    pub to: String,
    /// The updater stopped waiting for the startup check (`Outcome::Unconfirmed`): no
    /// updater removes the backup any more, so the first start that finds the new
    /// version's confirmation finishes the update.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unattended: bool,
}

/// Starts a program and reports when it exits; replaced in tests.
pub trait Launcher {
    fn launch(
        &self,
        executable: &Path,
        arguments: &[OsString],
        working_folder: &Path,
    ) -> io::Result<Box<dyn Started>>;
}

pub trait Started {
    /// `Some` once the program has exited.
    fn exited(&mut self) -> io::Result<Option<i32>>;
}

/// Starts real processes.
pub struct ProcessLauncher;

impl Launcher for ProcessLauncher {
    fn launch(
        &self,
        executable: &Path,
        arguments: &[OsString],
        working_folder: &Path,
    ) -> io::Result<Box<dyn Started>> {
        let folder = if working_folder.is_dir() {
            working_folder
        } else {
            executable.parent().unwrap_or(working_folder)
        };
        let child = Command::new(executable)
            .args(arguments)
            .current_dir(folder)
            .spawn()?;
        Ok(Box::new(ChildProcess(child)))
    }
}

struct ChildProcess(Child);

impl Started for ChildProcess {
    fn exited(&mut self) -> io::Result<Option<i32>> {
        Ok(self.0.try_wait()?.map(|status| status.code().unwrap_or(-1)))
    }
}

struct Log(Option<File>);

impl Log {
    fn open(work: &Path) -> Self {
        let path = work.join("update.log");
        if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 1024 * 1024) {
            let _ = fs::remove_file(&path);
        }
        Self(OpenOptions::new().create(true).append(true).open(path).ok())
    }

    fn line(&mut self, text: &str) {
        if let Some(file) = self.0.as_mut() {
            let _ = writeln!(file, "{} {text}", crate::client::unix_now());
        }
    }
}

/// The updater's entry point: applies the update the arguments describe and returns
/// the process exit status.
pub fn run(arguments: &[OsString]) -> u8 {
    match ApplyRequest::from_arguments(arguments) {
        Some(Ok(request)) => match apply(&request, &ProcessLauncher, Timings::default()) {
            Outcome::Updated { .. } | Outcome::Unconfirmed { .. } => 0,
            _ => 1,
        },
        _ => 2,
    }
}

/// Applies `request`. Every path ends with a running Midnight Explorer: the new
/// version, or the previous one again.
pub fn apply(request: &ApplyRequest, launcher: &dyn Launcher, timings: Timings) -> Outcome {
    let work = request.work();
    let mut log = Log::open(&work);
    log.line(&format!("updater started for {}", request.staged.display()));
    let outcome = apply_logged(request, launcher, timings, &mut log);
    log.line(&format!("outcome: {outcome:?}"));
    // Written before the previous version starts again, so that it can say why.
    let _ = write_json(&work.join("result.json"), &outcome);
    if matches!(
        outcome,
        Outcome::NotApplied { .. } | Outcome::RolledBack { .. }
    ) {
        relaunch_previous(request, launcher, &mut log);
    }
    outcome
}

fn not_applied(log: &mut Log, reason: String) -> Outcome {
    log.line(&format!("not applied: {reason}"));
    Outcome::NotApplied { reason }
}

fn relaunch_previous(request: &ApplyRequest, launcher: &dyn Launcher, log: &mut Log) {
    match BuildInfo::read(&request.installation) {
        Ok(info) => {
            let executable = request.installation.join(&info.binary);
            if let Err(error) =
                launcher.launch(&executable, &request.arguments, &request.working_folder)
            {
                log.line(&format!("cannot start {}: {error}", executable.display()));
            }
        }
        Err(error) => log.line(&format!("no previous version to start: {error}")),
    }
}

fn apply_logged(
    request: &ApplyRequest,
    launcher: &dyn Launcher,
    timings: Timings,
    log: &mut Log,
) -> Outcome {
    let work = request.work();
    let Some(platform) = PLATFORM else {
        return not_applied(log, "unsupported platform".into());
    };
    if request.staged.parent() != Some(work.join("staging").as_path()) {
        return not_applied(
            log,
            "the staged release is not in this installation's update folder".into(),
        );
    }
    let previous = match BuildInfo::read(&request.installation) {
        Ok(info) => info,
        Err(error) => return not_applied(log, error.to_string()),
    };
    // Every running copy holds the lock shared; wait until all have exited.
    let lock = match lock_file(&work).and_then(|file| {
        wait_until(timings.instances, timings.poll, || file.try_lock().is_ok())
            .then_some(file)
            .ok_or_else(|| io::Error::other("busy"))
    }) {
        Ok(lock) => lock,
        Err(_) => {
            return not_applied(log, "Another Midnight Explorer window from this folder is still open. Close it and choose Restart now again.".into(),
            );
        }
    };
    log.line("all copies have exited");
    let new = match BuildInfo::read(&request.staged)
        .and_then(|info| archive::verify_staged(&request.staged, &info.version, &platform))
    {
        Ok(info) => info,
        Err(error) => return not_applied(log, error.to_string()),
    };
    let from = previous.version.to_string();
    let to = new.version.to_string();
    let mut pending = Pending {
        from: from.clone(),
        to: to.clone(),
        unattended: false,
    };
    if let Err(error) = write_json(&work.join("pending.json"), &pending) {
        return not_applied(log, error.to_string());
    }
    let _ = fs::remove_file(work.join("confirmed"));
    let backup = work.join("backup");
    let failed = work.join("failed");
    for folder in [&backup, &failed] {
        if let Err(error) = remove_tree(folder).and_then(|()| fs::create_dir(folder)) {
            let _ = fs::remove_file(work.join("pending.json"));
            return not_applied(log, error.to_string());
        }
    }
    // 3. The swap.
    let old_entries = match move_entries(&request.installation, &backup, timings, log) {
        Ok(moved) => moved,
        Err(error) => {
            let _ = fs::remove_file(work.join("pending.json"));
            return not_applied(
                log,
                format!("The previous version's files cannot be moved aside: {error}"),
            );
        }
    };
    let new_entries = match move_entries(&request.staged, &request.installation, timings, log) {
        Ok(moved) => moved,
        Err(error) => {
            restore(&backup, &request.installation, &old_entries, timings, log);
            let _ = fs::remove_file(work.join("pending.json"));
            return not_applied(
                log,
                format!("The new version's files cannot be moved in: {error}"),
            );
        }
    };
    log.line(&format!("installed {to} in place of {from}"));
    // The staged copy is used up: the new version must not offer it again at its start.
    let _ = fs::remove_file(work.join("staging").join("ready.json"));
    let _ = remove_tree(&request.staged);
    drop(lock);
    // 4. The new version's startup check.
    let executable = request.installation.join(&new.binary);
    let confirmed = work.join("confirmed");
    let reason = match launcher.launch(&executable, &request.arguments, &request.working_folder) {
        Err(error) => format!("The new version could not be started: {error}"),
        Ok(mut started) => {
            let deadline = Instant::now() + timings.confirm;
            loop {
                if fs::read_to_string(&confirmed).is_ok_and(|text| text.trim() == to) {
                    log.line("the new version confirmed its start");
                    let _ = remove_tree(&backup);
                    let _ = remove_tree(&failed);
                    let _ = fs::remove_file(work.join("pending.json"));
                    return Outcome::Updated { from, to };
                }
                match started.exited() {
                    Ok(Some(code)) => {
                        break format!(
                            "The new version exited during its startup check (status {code})."
                        );
                    }
                    Ok(None) => {}
                    Err(error) => break format!("The new version cannot be watched: {error}"),
                }
                if Instant::now() >= deadline {
                    log.line("no confirmation in time; the backup is kept");
                    pending.unattended = true;
                    if let Err(error) = write_json(&work.join("pending.json"), &pending) {
                        log.line(&format!("cannot hand the confirmation over: {error}"));
                    }
                    return Outcome::Unconfirmed { from, to };
                }
                thread::sleep(timings.poll);
            }
        }
    };
    // 5. Rollback.
    log.line(&format!("rolling back: {reason}"));
    let lock = lock_file(&work)
        .ok()
        .filter(|file| wait_until(timings.instances, timings.poll, || file.try_lock().is_ok()));
    if lock.is_none() {
        log.line("the new version still holds the folder; rollback continues regardless");
    }
    if let Err(error) = move_named(&request.installation, &failed, &new_entries, timings, log) {
        log.line(&format!("cannot move the new version aside: {error}"));
    }
    restore(&backup, &request.installation, &old_entries, timings, log);
    let _ = fs::remove_file(work.join("pending.json"));
    drop(lock);
    Outcome::RolledBack { from, to, reason }
}

fn wait_until(limit: Duration, poll: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(poll);
    }
}

fn rename_retrying(from: &Path, to: &Path, timings: Timings) -> io::Result<()> {
    let deadline = Instant::now() + timings.rename;
    let mut pause = Duration::from_millis(50);
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error)
                if platform::is_transient_rename_error(&error) && Instant::now() < deadline =>
            {
                thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_secs(1));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Moves every entry of `from` except the work folder into `to`; on failure moves the
/// moved ones back. Returns the moved names.
fn move_entries(
    from: &Path,
    to: &Path,
    timings: Timings,
    log: &mut Log,
) -> io::Result<Vec<OsString>> {
    let mut names: Vec<OsString> = fs::read_dir(from)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<_>>()?;
    names.retain(|name| name != WORK_FOLDER);
    names.sort();
    move_named(from, to, &names, timings, log)?;
    Ok(names)
}

fn move_named(
    from: &Path,
    to: &Path,
    names: &[OsString],
    timings: Timings,
    log: &mut Log,
) -> io::Result<()> {
    for (index, name) in names.iter().enumerate() {
        if let Err(error) = rename_retrying(&from.join(name), &to.join(name), timings) {
            log.line(&format!(
                "cannot move {}: {error}",
                from.join(name).display()
            ));
            for done in names[..index].iter().rev() {
                let _ = rename_retrying(&to.join(done), &from.join(done), timings);
            }
            return Err(error);
        }
    }
    Ok(())
}

fn restore(
    backup: &Path,
    installation: &Path,
    names: &[OsString],
    timings: Timings,
    log: &mut Log,
) {
    for name in names {
        if let Err(error) = rename_retrying(&backup.join(name), &installation.join(name), timings) {
            log.line(&format!(
                "cannot restore {}: {error}",
                name.to_string_lossy()
            ));
        }
    }
}

/// What a starting copy learns about an update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupReport {
    /// This copy is the new version of an update in progress: it confirms once it runs.
    pub confirm: Option<String>,
    /// How the last update went, read once.
    pub outcome: Option<Outcome>,
}

/// Reads `pending.json` and `result.json` of `installation` at start. `version` is the
/// starting copy's own version. Removes what earlier updates left behind.
pub fn startup(installation: &Path, version: &str) -> StartupReport {
    let work = work_folder(installation);
    let mut report = StartupReport::default();
    let pending = fs::read(work.join("pending.json"))
        .ok()
        .and_then(|text| serde_json::from_slice::<Pending>(&text).ok());
    // A start of the new version after an unattended update (`Pending::unattended`)
    // that an earlier start already confirmed finishes it, as its updater would have.
    let finished = pending.as_ref().is_some_and(|pending| {
        pending.unattended
            && pending.to == version
            && fs::read_to_string(work.join("confirmed")).is_ok_and(|text| text.trim() == version)
    });
    if finished {
        let _ = fs::remove_file(work.join("pending.json"));
    }
    match pending {
        Some(pending) if pending.to == version && !finished => report.confirm = Some(pending.to),
        // The updater is still at work (or stopped mid-way): leave its folders alone.
        Some(_) if !finished => return report,
        _ => {
            let _ = remove_tree(&work.join("failed"));
            let _ = remove_tree(&work.join("updater"));
            // A backup without an update in progress belongs to a confirmed update
            // whose updater could not remove it.
            if work.join("confirmed").is_file() {
                let _ = remove_tree(&work.join("backup"));
                let _ = fs::remove_file(work.join("confirmed"));
            }
        }
    }
    if let Ok(text) = fs::read(work.join("result.json")) {
        report.outcome = serde_json::from_slice(&text).ok();
        let _ = fs::remove_file(work.join("result.json"));
    }
    report
}

/// The new version's startup check passed: its window shows with the user's settings.
pub fn confirm(installation: &Path, version: &str) -> io::Result<()> {
    let work = work_folder(installation);
    let path = work.join("confirmed");
    fs::write(&path, version)
}

/// Prepares "Restart now": copies the running executable to `.update/updater/` and
/// returns the updater's executable and its arguments.
pub fn prepare_restart(
    installation: &install::Installation,
    staged_folder: &str,
    current_executable: &Path,
    working_folder: PathBuf,
    arguments: Vec<OsString>,
) -> io::Result<(PathBuf, ApplyRequest)> {
    let staged = installation.staging().join(staged_folder);
    if !staged.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the staged update is missing",
        ));
    }
    let updater_folder = installation.work().join("updater");
    remove_tree(&updater_folder)?;
    fs::create_dir_all(&updater_folder)?;
    let updater = updater_folder.join(
        current_executable
            .file_name()
            .ok_or_else(|| io::Error::other("the executable has no name"))?,
    );
    fs::copy(current_executable, &updater)?;
    Ok((
        updater,
        ApplyRequest {
            installation: installation.folder.clone(),
            staged,
            working_folder,
            arguments,
        },
    ))
}

/// Starts the updater prepared by `prepare_restart`; the caller exits next.
pub fn start_updater(updater: &Path, request: &ApplyRequest) -> io::Result<()> {
    Command::new(updater)
        .args(request.to_arguments())
        .current_dir(request.work())
        .spawn()
        .map(drop)
}
