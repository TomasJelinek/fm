//! Staged child processes: the viewer scenarios with the installed plugin packages
//! ([`run`]), and scenarios that need a private environment ([`run_private`]).
//!
//! Plugin discovery is relative to the running executable: the viewer engine, the
//! Engine's archive catalogue, the comparator's image viewers and the thumbnail host all
//! look for `viewers/`, `filesystems/` and `thumbnailers/` beside it. The packages are
//! never staged beside the shared test binary (other runs would discover them).
//! Instead each `viewers_*` scenario runs in a child process started from a private
//! stage one level below the build folder, `<build>/acceptance-stage-<run>-<scenario>/`,
//! as `scripts/validate.sh` stages its native viewer tests: a link to this test
//! executable with copies of the installed packages beside it. A source checkout's
//! HTML-viewer AppArmor profile (`<checkout>/_build/*/viewers/midnight-viewer-html/…`,
//! `scripts/install-html-viewer-apparmor.sh`) covers that path as it covers a build.
//! `files_delete` runs in a child of this executable itself, without a stage.
//!
//! The child runs only that scenario on its own fixture and writes a result file; the
//! parent adopts its status, notes, screenshots and flags. A plugin crash or hang then
//! fails one scenario instead of the whole run. Every child gets private XDG cache,
//! config and data folders (Chromium sessions, spools and the Trash stay out of the
//! account's own), its own process group and a deadline counted from the scenario's
//! start. A guard kills that process group and removes the stage when the scenario ends,
//! however it ends; the runner calls [`stop_all`] for a scenario it gave up on, whose
//! thread never ends. A child whose parent is gone (Ctrl+C) removes its stage and kills
//! its own group.
//!
//! Files that already have every owner permission bit (executables and libraries,
//! most of the bytes) are hard-linked, everything else is copied, so removing a stage
//! cannot change the build's own package files whatever a scenario did to the stage's
//! permissions. The stage is removed as soon as the child exits.
use super::{
    Context, fixtures,
    report::{Flag, Shot, Status},
};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Set in a staged child to the one scenario it runs.
const STAGED: &str = "MIDNIGHT_ACCEPTANCE_STAGED";
/// Set in a staged child to the milliseconds it has, from its start, to report.
pub const LIMIT: &str = "MIDNIGHT_ACCEPTANCE_STAGED_LIMIT_MS";
/// Optional folder holding the `viewers/`, `filesystems/` and `thumbnailers/` packages
/// to stage; by default the build's profile folder (`_build/debug`).
const PACKAGES: &str = "MIDNIGHT_ACCEPTANCE_PLUGINS";
const RESULT: &str = "staged-result.tsv";
/// Stage folders are `<build>/acceptance-stage-<run>-<scenario>`.
const STAGE_PREFIX: &str = "acceptance-stage-";
/// The parent stops the child this long before the scenario's timeout, both counted
/// from the scenario's start; the child reports and leaves this long before that.
const PARENT_MARGIN: Duration = Duration::from_secs(20);
const CHILD_MARGIN: Duration = Duration::from_secs(25);

/// Run `body` with the installed plugin packages: in a staged child process, or
/// directly when this process is that child. `timeout` is the scenario's registered
/// timeout.
pub fn run(
    context: &mut Context,
    timeout: Duration,
    body: fn(&mut Context) -> Result<(), String>,
) -> Result<(), String> {
    staged(context, timeout, body, Kind::Plugins)
}

/// Run `body` in a child process of this executable with private XDG cache, config and
/// data folders (so the application's Trash is a private folder too) and no plugin
/// packages, or directly when this process is that child.
pub fn run_private(
    context: &mut Context,
    timeout: Duration,
    body: fn(&mut Context) -> Result<(), String>,
) -> Result<(), String> {
    staged(context, timeout, body, Kind::Private)
}

#[derive(Clone, Copy)]
enum Kind {
    Plugins,
    Private,
}

fn staged(
    context: &mut Context,
    timeout: Duration,
    body: fn(&mut Context) -> Result<(), String>,
    kind: Kind,
) -> Result<(), String> {
    let name = super::app::context_name();
    match std::env::var(STAGED) {
        Ok(staged) if staged == name => child(context, timeout, body),
        Ok(staged) => Err(format!(
            "SKIP: this staged process runs only {staged}, not {name}"
        )),
        Err(_) => parent(context, &name, timeout, kind),
    }
}

/// Package folders of `kind` (`viewers`, `thumbnailers`, …) beside this executable: in
/// a staged child, what its stage holds.
pub fn installed_packages(kind: &str) -> usize {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|folder| folder.join(kind)))
        .and_then(|folder| fs::read_dir(folder).ok())
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .count()
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------
// Child side

fn child(
    context: &mut Context,
    timeout: Duration,
    body: fn(&mut Context) -> Result<(), String>,
) -> Result<(), String> {
    watch_parent();
    let result_path = context.run_dir.join(RESULT);
    // A body that never returns cannot report through the runner: report here and
    // leave, so the parent gets a precise message instead of a killed process.
    let finished = Arc::new(AtomicBool::new(false));
    let watched = finished.clone();
    let deadline = context
        .options
        .staged_deadline
        .unwrap_or_else(|| context.started + timeout.saturating_sub(PARENT_MARGIN + CHILD_MARGIN));
    let limit = deadline.saturating_duration_since(context.started);
    let path = result_path.clone();
    let _ = std::thread::Builder::new()
        .name("staged-watchdog".into())
        .spawn(move || {
            while Instant::now() < deadline {
                if watched.load(Ordering::Acquire) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            if !watched.load(Ordering::Acquire) {
                let _ = fs::write(
                    &path,
                    format!(
                        "status\tFAIL\nmessage\t{}\n",
                        escape(&format!(
                            "the scenario did not finish within {limit:.1?} in its staged \
                             process"
                        ))
                    ),
                );
                eprintln!("staged scenario exceeded {limit:.1?}; leaving");
                stop_own_group();
                std::process::exit(3);
            }
        });
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(context)));
    finished.store(true, Ordering::Release);
    let (status, message) = match &outcome {
        // A passing scenario's message says what it skipped, if anything.
        Ok(Ok(())) => ("PASS", context.outcome.message.clone()),
        Ok(Err(message)) => {
            let (status, message) = Status::of_error(message);
            (status.label(), message)
        }
        Err(panic) => (
            "FAIL",
            panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                .unwrap_or_else(|| "panicked".to_owned()),
        ),
    };
    let mut text = format!("status\t{status}\nmessage\t{}\n", escape(&message));
    for note in &context.outcome.notes {
        text.push_str(&format!("note\t{}\n", escape(note)));
    }
    for shot in &context.outcome.shots {
        text.push_str(&format!(
            "shot\t{}\t{}\n",
            escape(&shot.file),
            escape(&shot.caption)
        ));
    }
    for flag in &context.outcome.flags {
        text.push_str(&format!(
            "flag\t{}\t{}\t{}\n",
            escape(&flag.shot),
            escape(flag.kind),
            escape(&flag.detail)
        ));
    }
    if let Err(error) = fs::write(&result_path, text) {
        eprintln!("cannot write {}: {error}", result_path.display());
    }
    match outcome {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// The parent holds the write end of this child's standard input, a pipe, until it has
/// reaped the child, and never writes: end of input means the parent is gone
/// (interrupted or killed), and nothing else would remove this child's stage or stop
/// its process group. A child started some other way (a terminal, `/dev/null`) is not
/// watched.
fn watch_parent() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if !fs::metadata("/proc/self/fd/0").is_ok_and(|input| input.file_type().is_fifo()) {
            return;
        }
    }
    let _ = std::thread::Builder::new()
        .name("staged-parent".into())
        .spawn(|| {
            let mut byte = [0_u8; 1];
            loop {
                match io::stdin().lock().read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            eprintln!("the parent of this staged process is gone; leaving");
            if let Some(stage) = own_stage() {
                let _ = fs::remove_dir_all(stage);
            }
            stop_own_group();
            std::process::exit(4);
        });
}

/// The stage this child runs from: the folder of its executable, when [`Stage::create`]
/// made it (`acceptance-stage-…`); never the build's own folder of a private child.
fn own_stage() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_path_buf();
    folder
        .file_name()?
        .to_str()?
        .starts_with(STAGE_PREFIX)
        .then_some(folder)
}

/// Kill this process's group (its plugin workers and itself) when this process leads
/// it, as a staged child started by [`parent`] does; otherwise do nothing.
fn stop_own_group() {
    #[cfg(unix)]
    if leads_own_group() {
        let _ = Command::new("/bin/sh")
            .args(["-c", "kill -s KILL 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Whether this process is its process group's leader (`/proc/self/stat`: the group
/// follows the state and the parent, after the parenthesised command name).
#[cfg(unix)]
fn leads_own_group() -> bool {
    fs::read_to_string("/proc/self/stat").is_ok_and(|stat| {
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(2)?.parse::<u32>().ok())
            == Some(std::process::id())
    })
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match characters.next() {
            Some('t') => output.push('\t'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }
    output
}

// ---------------------------------------------------------------------------------------
// Guards: staged children and stages that are still alive

/// Every staged child's process group and stage that has not been stopped and removed
/// yet. The runner reads it from its own thread when it abandons a scenario.
static LIVE: Mutex<Vec<Live>> = Mutex::new(Vec::new());
/// Set once the runner stopped everything: no staged child starts after that.
static STOPPED: AtomicBool = AtomicBool::new(false);
static NEXT: AtomicU64 = AtomicU64::new(1);

struct Live {
    id: u64,
    /// The child's process group (its process ID), until it was killed after the
    /// child exited or was stopped.
    group: Option<u32>,
    /// The stage folder this run created, until it is removed.
    stage: Option<PathBuf>,
}

fn live() -> MutexGuard<'static, Vec<Live>> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Stop every staged child (its whole process group) and remove every stage, from the
/// runner's thread, for a scenario it abandoned; later scenarios start no child.
pub fn stop_all() {
    let mut live = live();
    STOPPED.store(true, Ordering::Release);
    for entry in live.iter_mut() {
        if let Some(group) = entry.group.take() {
            kill_group(group);
        }
        if let Some(stage) = entry.stage.take() {
            let _ = fs::remove_dir_all(stage);
        }
    }
}

/// Kill process group `group` (the staged child and what it started in its group).
#[cfg(unix)]
fn kill_group(group: u32) {
    // The standard library cannot signal a process group and the suite has no unsafe
    // code: the shell's `kill` builtin can.
    let _ = Command::new("/bin/sh")
        .args(["-c", "kill -s KILL -- \"-$1\"", "sh"])
        .arg(group.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Kill process `group` and the processes it started.
#[cfg(not(unix))]
fn kill_group(group: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID"])
        .arg(group.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// One staged child's process group and stage: killed and removed when the guard
/// drops, however the scenario ends; [`stop_all`] does the same for an abandoned one.
///
/// A process group is killed while its leader is unreaped (its ID cannot be reused
/// then), or once, in the same critical section as the reaping, to end what the child
/// left in its group; never later, when the ID could belong to another program.
struct Guard {
    id: u64,
}

impl Guard {
    fn new() -> Result<Self, String> {
        let mut live = live();
        if STOPPED.load(Ordering::Acquire) {
            return Err("the run is stopping; no staged process starts".into());
        }
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        live.push(Live {
            id,
            group: None,
            stage: None,
        });
        Ok(Self { id })
    }

    fn entry<'a>(&self, live: &'a mut MutexGuard<'static, Vec<Live>>) -> Option<&'a mut Live> {
        if STOPPED.load(Ordering::Acquire) {
            return None;
        }
        live.iter_mut().find(|entry| entry.id == self.id)
    }

    /// Remove `root`, a stage folder this run just created, with this guard.
    fn stage(&self, root: &Path) -> Result<(), String> {
        let mut live = live();
        match self.entry(&mut live) {
            Some(entry) => {
                entry.stage = Some(root.to_path_buf());
                Ok(())
            }
            None => {
                let _ = fs::remove_dir_all(root);
                Err("the run is stopping; no staged process starts".into())
            }
        }
    }

    /// Start `command` and record its process group, both under the lock: [`stop_all`]
    /// either prevents the start or stops the child.
    fn spawn(&self, command: &mut Command) -> Result<Child, String> {
        let mut live = live();
        let Some(entry) = self.entry(&mut live) else {
            return Err("the run is stopping; no staged process starts".into());
        };
        let child = command.spawn().map_err(|error| error.to_string())?;
        entry.group = Some(child.id());
        Ok(child)
    }

    /// `try_wait`; once the child has exited, also kill what it left in its group.
    fn poll(&self, process: &mut Child) -> io::Result<Option<ExitStatus>> {
        let mut live = live();
        let status = process.try_wait()?;
        if status.is_some()
            && let Some(group) = live
                .iter_mut()
                .find(|entry| entry.id == self.id)
                .and_then(|entry| entry.group.take())
        {
            kill_group(group);
        }
        Ok(status)
    }

    /// Kill the child's process group now and reap the child.
    fn kill(&self, process: &mut Child) {
        {
            let mut live = live();
            match live
                .iter_mut()
                .find(|entry| entry.id == self.id)
                .and_then(|entry| entry.group.take())
            {
                Some(group) => kill_group(group),
                None => {
                    let _ = process.kill();
                }
            }
        }
        let _ = process.wait();
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let entry = {
            let mut live = live();
            live.iter()
                .position(|entry| entry.id == self.id)
                .map(|index| live.remove(index))
        };
        // The child, if any, was not reaped where its group is still recorded.
        if let Some(entry) = entry {
            if let Some(group) = entry.group {
                kill_group(group);
            }
            if let Some(stage) = entry.stage {
                let _ = fs::remove_dir_all(stage);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Parent side

/// Where the packages come from: `MIDNIGHT_ACCEPTANCE_PLUGINS`, or the profile folder
/// of this test executable (`<target>/debug/deps/acceptance-…` → `<target>/debug`).
fn package_source() -> Result<PathBuf, String> {
    let source = match std::env::var_os(PACKAGES) {
        Some(folder) => PathBuf::from(folder),
        None => {
            let executable = std::env::current_exe()
                .map_err(|error| format!("cannot locate this test executable: {error}"))?;
            executable
                .parent()
                .and_then(Path::parent)
                .ok_or("the test executable has no profile folder")?
                .to_path_buf()
        }
    };
    if !source.join("viewers").is_dir() || !source.join("filesystems").is_dir() {
        return Err(format!(
            "SKIP: no installed plugin packages in {} (viewers/ and filesystems/); build \
             them first (./build.sh) or name their folder in {PACKAGES}",
            source.display()
        ));
    }
    Ok(source)
}

/// The running binary, readable even after its file was replaced (Linux).
const RUNNING: &str = "/proc/self/exe";

/// This test executable as a file to link or copy, and its name: its own path, or,
/// when a rebuild replaced the file while this run goes on, the running binary.
fn this_executable() -> Result<(PathBuf, String), String> {
    let path = std::env::current_exe()
        .map_err(|error| format!("cannot locate this test executable: {error}"))?;
    let name = path
        .file_name()
        .ok_or("the test executable has no name")?
        .to_string_lossy()
        .trim_end_matches(" (deleted)")
        .to_owned();
    if path.is_file() {
        Ok((path, name))
    } else if Path::new(RUNNING).is_file() {
        Ok((PathBuf::from(RUNNING), name))
    } else {
        Err(format!("{} is gone and cannot be staged", path.display()))
    }
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// The build folder of this test executable: `<build>/<profile>/deps/acceptance-…`.
fn build_folder() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate this test executable: {error}"))?;
    executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} is not in a build folder", executable.display()))
}

/// A private installation: this executable with the packages beside it, removed when
/// it drops (and by its guard).
struct Stage {
    root: PathBuf,
    executable: PathBuf,
    linked: usize,
    copied: usize,
    copied_bytes: u64,
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Stage {
    /// A new folder `<build>/acceptance-stage-<run>-<scenario>` (never an existing one),
    /// registered with `guard` before anything is copied into it. `<build>` is the
    /// `_build` folder holding the packages' installation (`_build/try-1221`), so the
    /// stage sits beside it where the same AppArmor profile pattern covers it, or else
    /// this executable's build folder.
    fn create(guard: &Guard, source: &Path, run: &str, scenario: &str) -> Result<Self, String> {
        let build = source
            .parent()
            .filter(|parent| parent.file_name().is_some_and(|name| name == "_build"))
            .map_or_else(build_folder, |parent| Ok(parent.to_path_buf()))?;
        let root = fixtures::create_fresh(&build, &format!("{STAGE_PREFIX}{run}-{scenario}"))?;
        guard.stage(&root)?;
        private(&root)?;
        let mut stage = Self {
            root: root.clone(),
            executable: PathBuf::new(),
            linked: 0,
            copied: 0,
            copied_bytes: 0,
        };
        let (executable, name) = this_executable()?;
        stage.executable = root.join(name);
        stage.place(&executable, &stage.executable.clone())?;
        for package in ["viewers", "thumbnailers"] {
            let from = source.join(package);
            if from.is_dir() {
                stage.tree(&from, &root.join(package))?;
            }
        }
        // The native and archive filesystem plugins; network providers are not needed.
        let filesystems = root.join("filesystems");
        private_folder(&filesystems)?;
        for entry in fs::read_dir(source.join("filesystems"))
            .map_err(|error| format!("{}: {error}", source.display()))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("network.") {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stage.tree(&entry.path(), &filesystems.join(&name))?;
            }
        }
        Ok(stage)
    }

    fn tree(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        private_folder(to)?;
        for entry in fs::read_dir(from).map_err(|error| format!("{}: {error}", from.display()))? {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            let target = to.join(entry.file_name());
            if kind.is_dir() {
                self.tree(&entry.path(), &target)?;
            } else if kind.is_file() {
                self.place(&entry.path(), &target)?;
            } else {
                return Err(format!(
                    "{} is neither a file nor a folder; plugin packages contain only those",
                    entry.path().display()
                ));
            }
        }
        Ok(())
    }

    /// Hard-link a file that already has every owner permission bit, copy the rest.
    fn place(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        let metadata =
            fs::metadata(from).map_err(|error| format!("{}: {error}", from.display()))?;
        if owner_complete(&metadata) && fs::hard_link(from, to).is_ok() {
            self.linked += 1;
            return Ok(());
        }
        let bytes = fs::copy(from, to)
            .map_err(|error| format!("cannot copy {} to the stage: {error}", from.display()))?;
        self.copied += 1;
        self.copied_bytes += bytes;
        Ok(())
    }
}

#[cfg(unix)]
fn owner_complete(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o700 == 0o700
}

#[cfg(not(unix))]
fn owner_complete(metadata: &fs::Metadata) -> bool {
    // `remove_tree` only clears the read-only attribute there; link writable files.
    !metadata.permissions().readonly()
}

/// Make an existing folder one only its owner can use: plugin trust checks reject
/// group- or world-writable ancestors, whatever the umask.
fn private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Create (if needed) a folder only its owner can use.
fn private_folder(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("{}: {error}", path.display()))?;
    private(path)
}

/// Private XDG cache, config and data folders in `folder` for the child's environment:
/// Chromium's sessions, the archive spool and the Trash stay inside it.
fn private_xdg(folder: &Path) -> Result<[(&'static str, PathBuf); 3], String> {
    let variables = [
        ("XDG_CACHE_HOME", folder.join("cache")),
        ("XDG_CONFIG_HOME", folder.join("config")),
        ("XDG_DATA_HOME", folder.join("data")),
    ];
    private_folder(folder)?;
    for (_, path) in &variables {
        private_folder(path)?;
    }
    Ok(variables)
}

/// The run folder the child created in `root` (the only folder there), once it got
/// that far.
fn child_run(root: &Path) -> Option<PathBuf> {
    let mut folders = fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path());
    let first = folders.next()?;
    folders.next().is_none().then_some(first)
}

fn parent(context: &mut Context, name: &str, timeout: Duration, kind: Kind) -> Result<(), String> {
    // Counted from the scenario's start, as the runner counts its timeout.
    let deadline = context.started + timeout.saturating_sub(PARENT_MARGIN);
    let guard = Guard::new()?;
    let folder = context.work.join("staged");
    let child_root = folder.join("run");
    let console = folder.join("console.log");
    for path in [&folder, &child_root] {
        fs::create_dir(path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    let (stage, executable, xdg) = match kind {
        Kind::Plugins => {
            let source = package_source()?;
            let prepared = Instant::now();
            let run = context
                .run_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let stage = Stage::create(&guard, &source, &run, name)?;
            context.note(format!(
                "Staged installation {} from {}: {} file(s) linked, {} copied ({:.1} MiB) in \
                 {:.2} s.",
                stage.root.display(),
                source.display(),
                stage.linked,
                stage.copied,
                stage.copied_bytes as f64 / 1_048_576.0,
                prepared.elapsed().as_secs_f64()
            ));
            let (executable, xdg) = (stage.executable.clone(), stage.root.join("xdg"));
            (Some(stage), executable, xdg)
        }
        Kind::Private => {
            let (executable, name) = this_executable()?;
            // A rebuild replaced this executable meanwhile: run a private copy of the
            // running one.
            let executable = if executable == Path::new(RUNNING) {
                let copy = folder.join(name);
                fs::copy(&executable, &copy).map_err(|error| io_error(&copy, error))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&copy, fs::Permissions::from_mode(0o700))
                        .map_err(|error| io_error(&copy, error))?;
                }
                copy
            } else {
                executable
            };
            (None, executable, folder.join("xdg"))
        }
    };
    let xdg = private_xdg(&xdg)?;
    let log = fs::File::create(&console)
        .map_err(|error| format!("cannot create {}: {error}", console.display()))?;
    let errors = log.try_clone().map_err(|error| error.to_string())?;
    let scale = match context.fixture.scale {
        fixtures::Scale::Smoke => "smoke",
        fixtures::Scale::Full => "full",
    };
    let child_limit = deadline
        .saturating_duration_since(Instant::now())
        .saturating_sub(CHILD_MARGIN);
    let mut command = Command::new(&executable);
    command
        .env(STAGED, name)
        .env(LIMIT, child_limit.as_millis().to_string())
        .env("MIDNIGHT_ACCEPTANCE_FILTER", name)
        .env("MIDNIGHT_ACCEPTANCE_RUN_DIR", &child_root)
        .env("MIDNIGHT_ACCEPTANCE_SCALE", scale)
        .env(
            "MIDNIGHT_ACCEPTANCE_KEEP_FIXTURE",
            if context.options.keep_fixture {
                "1"
            } else {
                "0"
            },
        )
        .envs(
            xdg.iter()
                .map(|(variable, path)| (*variable, path.as_os_str())),
        )
        // Held open, never written: its end tells the child that this process is gone.
        .stdin(Stdio::piped())
        .stdout(log)
        .stderr(errors);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let started = Instant::now();
    let mut process = guard.spawn(&mut command).map_err(|error| {
        format!(
            "cannot start the staged process {}: {error}",
            executable.display()
        )
    })?;
    let status = loop {
        match guard.poll(&mut process) {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                guard.kill(&mut process);
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(format!("waiting for the staged process: {error}")),
        }
    };
    drop(stage);
    let elapsed = started.elapsed();
    let relative = console
        .strip_prefix(&context.run_dir)
        .unwrap_or(&console)
        .display()
        .to_string();
    context.note(format!(
        "Ran in a staged child process for {:.1} s ({}); its console: {relative}.",
        elapsed.as_secs_f64(),
        match status {
            Some(status) => status.to_string(),
            None => "killed at its deadline".to_owned(),
        }
    ));
    let result = child_run(&child_root).and_then(|run| adopt(context, name, &run));
    let result = match (result, status) {
        (Some(result), _) => result,
        (None, None) => Err(format!(
            "the staged process did not finish within {:?} of the scenario's start; console \
             tail:\n{}",
            timeout.saturating_sub(PARENT_MARGIN),
            tail(&console, 24)
        )),
        (None, Some(status)) => Err(format!(
            "the staged process ended ({status}) without a result; console tail:\n{}",
            tail(&console, 24)
        )),
    };
    if result.is_ok() && !context.options.keep_fixture {
        let _ = fixtures::remove_tree(&child_root);
    }
    result
}

/// Adopt the child's screenshots, notes and flags; `None` without a result file.
fn adopt(context: &mut Context, name: &str, child_run: &Path) -> Option<Result<(), String>> {
    let text = fs::read_to_string(child_run.join(RESULT)).ok();
    let mut status = String::from("FAIL");
    let mut message = String::new();
    let mut adopted: Vec<String> = Vec::new();
    for line in text.as_deref().unwrap_or_default().lines() {
        let fields: Vec<String> = line.split('\t').map(unescape).collect();
        match fields.first().map(String::as_str) {
            Some("status") if fields.len() > 1 => status = fields[1].clone(),
            Some("message") if fields.len() > 1 => message = fields[1].clone(),
            Some("note") if fields.len() > 1 => context.note(fields[1].clone()),
            Some("shot") if fields.len() > 2 => {
                if move_shot(context, child_run, &fields[1]) {
                    adopted.push(fields[1].clone());
                    context.outcome.shots.push(Shot {
                        file: fields[1].clone(),
                        caption: fields[2].clone(),
                        flags: 0,
                    });
                }
            }
            Some("flag") if fields.len() > 3 => {
                let kind: &'static str = Box::leak(fields[2].clone().into_boxed_str());
                if let Some(shot) = context
                    .outcome
                    .shots
                    .iter_mut()
                    .find(|shot| shot.file == fields[1])
                {
                    shot.flags += 1;
                }
                context.outcome.flags.push(Flag {
                    shot: fields[1].clone(),
                    kind,
                    detail: fields[3].clone(),
                });
            }
            _ => {}
        }
    }
    // Screenshots taken before a crash or a kill still help a reviewer.
    let folder = child_run.join("shots").join(name);
    if let Ok(entries) = fs::read_dir(&folder) {
        let mut rest: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        rest.sort();
        for path in rest {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            let relative = format!("shots/{name}/{file_name}");
            if !adopted.contains(&relative) && move_shot(context, child_run, &relative) {
                context.outcome.shots.push(Shot {
                    file: relative,
                    caption: format!("{file_name} (recorded before the staged process ended)"),
                    flags: 0,
                });
            }
        }
    }
    text.as_ref()?;
    Some(match status.as_str() {
        "PASS" | "WARN" => {
            if !message.is_empty() {
                context.outcome.message = message;
            }
            Ok(())
        }
        "SKIP" => Err(format!("SKIP: {message}")),
        "PENDING" => Err(format!("PENDING: {message}")),
        _ => Err(if message.is_empty() {
            "the staged scenario failed without a message".to_owned()
        } else {
            message
        }),
    })
}

fn move_shot(context: &Context, child_run: &Path, relative: &str) -> bool {
    let from = child_run.join(relative);
    let to = context.run_dir.join(relative);
    if let Some(parent) = to.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    fs::rename(&from, &to).is_ok() || fs::copy(&from, &to).is_ok()
}

fn tail(path: &Path, lines: usize) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
