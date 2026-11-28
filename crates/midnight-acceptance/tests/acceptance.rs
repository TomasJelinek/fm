//! Opt-in end-to-end acceptance suite: the real MainWindow, Presenter and Engine on the
//! native filesystem, driven by synthetic keyboard and pointer input on software-rendered
//! windows, against a generated fixture tree.
//!
//! This target needs the `acceptance` feature and has its own `main` (`harness = false`):
//! parallel scenarios use private worker processes, and shared-desktop scenarios run
//! exclusively, each under a watchdog. Run it with `./build.sh --acceptance` or
//! `scripts/acceptance.sh`; see docs/developer/acceptance.md.
// Deny, not forbid: the slint! fixture windows of the UI polish gate's red cases are
// generated bindings that carry their own narrow allowance (CODE_STYLE.md).
#![deny(unsafe_code)]
#![allow(clippy::too_many_lines)]

#[path = "acceptance/app.rs"]
mod app;
#[path = "acceptance/encode.rs"]
mod encode;
#[path = "acceptance/files_support.rs"]
mod files_support;
#[path = "acceptance/fixtures.rs"]
mod fixtures;
#[path = "acceptance/input.rs"]
mod input;
#[path = "acceptance/inspect.rs"]
mod inspect;
#[path = "acceptance/launches.rs"]
mod launches;
#[cfg(windows)]
#[path = "acceptance/native_windows.rs"]
mod native_windows;
#[path = "acceptance/options_support.rs"]
mod options_support;
#[path = "acceptance/panel.rs"]
mod panel;
#[path = "acceptance/report.rs"]
mod report;
#[path = "acceptance/review.rs"]
mod review;
#[path = "acceptance/scenarios/mod.rs"]
mod scenarios;
#[path = "acceptance/schedule.rs"]
mod schedule;
#[path = "acceptance/timing.rs"]
mod timing;
#[path = "acceptance/tool.rs"]
mod tool;
#[path = "acceptance/tools_support.rs"]
mod tools_support;
#[path = "acceptance/viewers_stage.rs"]
mod viewers_stage;
#[path = "acceptance/viewers_support.rs"]
mod viewers_support;
#[cfg(target_os = "linux")]
#[path = "acceptance/worker_display.rs"]
mod worker_display;

use report::{Outcome, Status};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// What every scenario receives.
pub struct Context {
    /// This parent is launching a private, software-rendered parallel worker.
    parallel: bool,
    /// The shared, read-only fixture tree. Scenarios that change files copy what they
    /// need into [`Context::work`] first.
    pub fixture: Arc<fixtures::Fixture>,
    /// This run's output directory (`_build/acceptance/<run>`), which the run created.
    pub run_dir: PathBuf,
    /// A private, initially empty scratch folder for this scenario.
    pub work: PathBuf,
    pub options: Arc<Options>,
    /// When the scenario started: its registered timeout counts from here.
    pub started: Instant,
    /// Screenshots, notes and review flags gathered while the scenario runs.
    pub outcome: Outcome,
}

impl Context {
    pub fn note(&mut self, text: impl Into<String>) {
        self.outcome.notes.push(text.into());
    }
}

pub type Run = fn(&mut Context) -> Result<(), String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    Parallel,
    /// The entire scenario already runs in a private staged process.
    ParallelStaged,
    /// Uses installed plugins, which can need a real X server for Chromium on Linux.
    ParallelPlugins,
    Exclusive,
}

impl Isolation {
    /// These native viewer checks enforce startup or exit deadlines that
    /// competing Windows workers can consume. Linux can use private X displays.
    const fn timed_plugin() -> Self {
        if cfg!(windows) {
            Self::Exclusive
        } else {
            Self::ParallelPlugins
        }
    }

    fn effective(self) -> Self {
        #[cfg(target_os = "linux")]
        if self == Self::ParallelPlugins && worker_display::needs_shared_desktop() {
            return Self::Exclusive;
        }
        self
    }

    fn label(self) -> &'static str {
        match self.effective() {
            Self::Exclusive => "exclusive",
            Self::Parallel | Self::ParallelStaged | Self::ParallelPlugins => "parallel",
        }
    }
}

pub struct Scenario {
    pub isolation: Isolation,
    pub name: &'static str,
    pub area: &'static str,
    pub summary: &'static str,
    pub run: Run,
    pub timeout: Duration,
}

pub struct Options {
    pub jobs: usize,
    pub filters: Vec<String>,
    pub scale: fixtures::Scale,
    /// The folder in which the run creates its own folder (`<run_root>/<run id>`).
    pub run_root: PathBuf,
    /// Whether `run_root` is the default `_build/acceptance`, where run folders of
    /// earlier versions (without the run marker) are pruned too.
    pub default_root: bool,
    pub keep_fixture: bool,
    pub strict_review: bool,
    /// Review every theme instead of the representative subset.
    pub all_themes: bool,
    pub list: bool,
    /// In a staged child process: when it must have reported (see `viewers_stage`).
    pub staged_deadline: Option<Instant>,
}

fn usage() -> &'static str {
    "Usage: acceptance [FILTER...] [--list] [--jobs 1..4] [--scale smoke|full] [--run-dir DIR] \
     [--keep-fixture] [--strict-review] [--all-themes] [--stress]\n\
     FILTER selects scenarios whose name contains it. --run-dir names an absolute \
     folder in which the run creates its own new folder (default _build/acceptance). \
     --stress runs the opt-in stress sizes. Environment equivalents: \
     MIDNIGHT_ACCEPTANCE_FILTER (comma-separated), MIDNIGHT_ACCEPTANCE_SCALE, \
     MIDNIGHT_ACCEPTANCE_RUN_DIR, MIDNIGHT_ACCEPTANCE_JOBS (1..4), MIDNIGHT_ACCEPTANCE_KEEP_FIXTURE=1, \
     MIDNIGHT_ACCEPTANCE_STRICT_REVIEW=1, MIDNIGHT_ACCEPTANCE_ALL_THEMES=1, \
     MIDNIGHT_ACCEPTANCE_STRESS=1 (staged children read only this one)."
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

fn job_count(value: &std::ffi::OsStr) -> Result<usize, String> {
    value
        .to_str()
        .and_then(|value| value.parse().ok())
        .filter(|n| (1..=4).contains(n))
        .ok_or_else(|| format!("jobs must be an integer from 1 to 4, got {value:?}"))
}

/// A folder named for the run folders: absolute and not a filesystem root. A run never
/// uses it as its own folder, it creates a new one inside (`report::create_run`).
fn run_root(value: std::ffi::OsString, source: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() {
        return Err(format!(
            "{source} is empty; name an absolute folder for the run folders"
        ));
    }
    if !path.is_absolute() {
        return Err(format!(
            "{source} {} is relative; name an absolute folder for the run folders",
            path.display()
        ));
    }
    if path.parent().is_none() {
        return Err(format!(
            "{source} {} is a filesystem root; name a folder for the run folders",
            path.display()
        ));
    }
    Ok(path)
}

fn options() -> Result<Options, String> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let staged_deadline = std::env::var(viewers_stage::LIMIT)
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|millis| Instant::now() + Duration::from_millis(millis));
    let mut options = Options {
        jobs: if cfg!(any(windows, target_os = "linux")) {
            std::thread::available_parallelism().map_or(1, |n| n.get().min(4))
        } else {
            1
        },
        filters: std::env::var("MIDNIGHT_ACCEPTANCE_FILTER")
            .map(|value| {
                value
                    .split(',')
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        scale: fixtures::Scale::Full,
        run_root: repository.join("_build/acceptance"),
        default_root: true,
        keep_fixture: env_flag("MIDNIGHT_ACCEPTANCE_KEEP_FIXTURE"),
        strict_review: env_flag("MIDNIGHT_ACCEPTANCE_STRICT_REVIEW"),
        all_themes: env_flag("MIDNIGHT_ACCEPTANCE_ALL_THEMES"),
        list: false,
        staged_deadline,
    };
    let mut jobs = std::env::var_os("MIDNIGHT_ACCEPTANCE_JOBS");
    if let Some(value) = std::env::var_os("MIDNIGHT_ACCEPTANCE_RUN_DIR") {
        options.run_root = run_root(value, "MIDNIGHT_ACCEPTANCE_RUN_DIR")?;
        options.default_root = false;
    }
    let mut scale = std::env::var("MIDNIGHT_ACCEPTANCE_SCALE").ok();
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let Some(argument) = argument.to_str() else {
            return Err(format!(
                "invalid argument {}\n{}",
                argument.to_string_lossy(),
                usage()
            ));
        };
        match argument {
            "--list" => options.list = true,
            "--keep-fixture" => options.keep_fixture = true,
            "--strict-review" => options.strict_review = true,
            "--all-themes" => options.all_themes = true,
            // `viewers_stage::stress` reads the argument and `MIDNIGHT_ACCEPTANCE_STRESS`.
            "--stress" => {}
            "--jobs" => jobs = Some(arguments.next().ok_or("--jobs needs a value")?),
            "--scale" => {
                let value = arguments.next().ok_or("--scale needs a value")?;
                scale = Some(value.to_string_lossy().into_owned());
            }
            "--run-dir" => {
                let value = arguments.next().ok_or("--run-dir needs a value")?;
                options.run_root = run_root(value, "--run-dir")?;
                options.default_root = false;
            }
            "--help" | "-h" => return Err(usage().to_owned()),
            // libtest-style flags some tools add; this runner has no use for them.
            "--nocapture" | "--quiet" | "-q" | "--color" => {}
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option {flag}\n{}", usage()));
            }
            filter => options.filters.push(filter.to_owned()),
        }
    }
    if let Some(value) = jobs {
        options.jobs = job_count(&value)?;
    }
    options.scale = match scale.as_deref() {
        None | Some("full") => fixtures::Scale::Full,
        Some("smoke") => fixtures::Scale::Smoke,
        Some(other) => return Err(format!("unknown scale {other}; use smoke or full")),
    };
    Ok(options)
}

fn main() -> std::process::ExitCode {
    // Started by the application as a scenario's harmless program (`tool.rs`).
    if let Some(code) = tool::run_if_requested() {
        return code;
    }
    // One line per panic on the console; the full message (with the harness's
    // diagnostics) goes to the scenario's result and the report.
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("panic");
        let thread = std::thread::current();
        eprintln!(
            "[{}] panicked at {}: {}",
            thread.name().unwrap_or("main"),
            info.location()
                .map_or_else(String::new, ToString::to_string),
            message.lines().next().unwrap_or_default()
        );
    }));
    let options = match options() {
        Ok(options) => Arc::new(options),
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(2);
        }
    };
    // Modules register scenarios of their own: each name names one work folder.
    let mut names = std::collections::HashSet::new();
    if let Some(twice) = scenarios::all()
        .iter()
        .find(|scenario| !names.insert(scenario.name))
    {
        eprintln!("two scenarios are named {}", twice.name);
        return std::process::ExitCode::from(2);
    }
    let child_name = viewers_stage::child_scenario();
    let selected: Vec<&Scenario> = scenarios::all()
        .iter()
        .filter(|scenario| {
            if let Some(name) = &child_name {
                return scenario.name == name;
            }
            options.filters.is_empty()
                || options
                    .filters
                    .iter()
                    .any(|filter| scenario.name.contains(filter.as_str()))
        })
        .collect();
    if options.list {
        for scenario in &selected {
            println!(
                "{:<28} {:<12} {:<10} {}",
                scenario.name,
                scenario.area,
                scenario.isolation.label(),
                scenario.summary
            );
        }
        return std::process::ExitCode::SUCCESS;
    }
    if selected.is_empty() {
        eprintln!("no scenario matches {:?}", options.filters);
        return std::process::ExitCode::from(2);
    }
    let started = Instant::now();
    // The run's own new folder: everything the run deletes later is inside it.
    let run_dir = match report::create_run(&options.run_root) {
        Ok(run_dir) => run_dir,
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    report::prune_old_runs(&run_dir, options.default_root);
    println!("Acceptance run: {}", run_dir.display());
    let fixture_root = run_dir.join("fixture");
    let generated = Instant::now();
    let fixture = match fixtures::Fixture::generate(&fixture_root, options.scale) {
        Ok(fixture) => Arc::new(fixture),
        Err(error) => {
            eprintln!("fixture generation failed: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!(
        "Fixture: {} ({:?} scale, seed {:#x}, {:.1} s)",
        fixture.root.display(),
        fixture.scale,
        fixtures::SEED,
        generated.elapsed().as_secs_f64()
    );
    let (results, abandoned) = schedule::run(&selected, &fixture, &run_dir, &options, started);
    let mut failed = results
        .iter()
        .filter(|(_, outcome)| outcome.status.is_failure(options.strict_review))
        .count();
    let index = report::write(&run_dir, &fixture, &results, started.elapsed(), &options);
    report::print_summary(&results, started.elapsed(), &index, options.strict_review);
    if abandoned {
        // A hung scenario thread cannot be joined; leave without unwinding it. Its
        // staged child and stage were stopped when it timed out; a thread that kept
        // going since may have started another.
        viewers_stage::stop_all();
    }
    if !options.keep_fixture && failed == 0 {
        let _ = fixtures::remove_tree(&fixture_root);
        let _ = fixtures::remove_tree(&run_dir.join("work"));
    }
    // A kept run folder (failures, --keep-fixture) can be deleted as a whole: the
    // fixture's read-only entries get owner write permission back now that no scenario
    // needs them.
    let locked = deletable(&run_dir);
    if !locked.is_empty() {
        eprintln!(
            "The run folder {} cannot be deleted: these entries keep owner write permission \
             off: {locked:?}",
            run_dir.display()
        );
        failed += 1;
    }
    if abandoned {
        std::process::exit(1);
    }
    if failed == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

/// How long the runner waits, after a staged scenario's timeout, for its thread to come
/// back. Its parent side stops the staged child before the timeout (`viewers_stage`);
/// a thread that returns within this time has ended, and the run goes on.
const STAGED_GRACE: Duration = Duration::from_secs(60);

/// Run one scenario on its own thread under its timeout; its outcome, and whether its
/// thread hung and was abandoned (the run then stops).
fn run_one(
    scenario: &'static Scenario,
    fixture: &Arc<fixtures::Fixture>,
    run_dir: &Path,
    options: &Arc<Options>,
    parallel: bool,
) -> (Outcome, bool) {
    // A new folder in this run's new folder: never one that existed before.
    let work = run_dir.join("work").join(scenario.name);
    if let Err(error) =
        std::fs::create_dir_all(run_dir.join("work")).and_then(|()| std::fs::create_dir(&work))
    {
        return (
            Outcome::with(
                Status::Fail,
                format!("cannot create {}: {error}", work.display()),
            ),
            false,
        );
    }
    let started = Instant::now();
    let work_folder = work.clone();
    let context = Context {
        parallel,
        fixture: fixture.clone(),
        run_dir: run_dir.to_path_buf(),
        work,
        options: options.clone(),
        started,
        outcome: Outcome::default(),
    };
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name(scenario.name.to_owned())
        .stack_size(16 << 20)
        .spawn(move || {
            let mut context = context;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                viewers_stage::run_scheduled(&mut context, scenario)
            }));
            let mut outcome = std::mem::take(&mut context.outcome);
            outcome.notes.extend(timing::take_notes());
            match result {
                Ok(Ok(())) => {
                    if outcome.status == Status::Pass && !outcome.flags.is_empty() {
                        outcome.status = Status::Warn;
                    }
                }
                Ok(Err(message)) => {
                    (outcome.status, outcome.message) = Status::of_error(&message);
                }
                Err(panic) => {
                    outcome.status = Status::Fail;
                    outcome.message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                        .unwrap_or_else(|| "panicked".to_owned());
                }
            }
            let _ = sender.send(outcome);
        });
    let (mut outcome, hung) = match spawned {
        Err(error) => (
            Outcome::with(
                Status::Fail,
                format!("cannot start the scenario thread: {error}"),
            ),
            false,
        ),
        Ok(handle) => match receiver.recv_timeout(scenario.timeout) {
            Ok(outcome) => {
                let _ = handle.join();
                (outcome, false)
            }
            Err(_) => {
                // A staged scenario (it made its `staged` folder) stops its child at its
                // own deadline, before this timeout: a thread that comes back after that
                // has ended, and the run goes on. An in-process one cannot be stopped.
                let late = work_folder
                    .join("staged")
                    .is_dir()
                    .then(|| receiver.recv_timeout(STAGED_GRACE).ok())
                    .flatten();
                match late {
                    Some(late) => {
                        let _ = handle.join();
                        let mut outcome = Outcome::with(
                            Status::Timeout,
                            format!(
                                "no result within {:?}; the staged scenario ended {:.1} s \
                                 later and the run went on. Its own result: {} {}",
                                scenario.timeout,
                                started
                                    .elapsed()
                                    .saturating_sub(scenario.timeout)
                                    .as_secs_f64(),
                                late.status.label(),
                                late.message
                            ),
                        );
                        outcome.notes = late.notes;
                        outcome.shots = late.shots;
                        outcome.flags = late.flags;
                        (outcome, false)
                    }
                    None => {
                        // The thread is abandoned, so its guards never run: stop its
                        // staged child's process group and remove its stage here.
                        viewers_stage::stop_all();
                        (
                            Outcome::with(
                                Status::Timeout,
                                format!("no result within {:?}", scenario.timeout),
                            ),
                            true,
                        )
                    }
                }
            }
        },
    };
    outcome.duration = started.elapsed();
    // Whatever the scenario left read-only (fixture copies, unreadable folders, a
    // killed staged child's run), its work folder stays deletable.
    let locked = deletable(&work_folder);
    if !locked.is_empty() {
        let message = format!(
            "the work folder cannot be deleted: {} entr(ies) keep owner write permission \
             off after the restore: {locked:?}",
            locked.len()
        );
        if outcome.status.is_failure(false) {
            outcome.message.push_str(&format!("\n{message}"));
        } else {
            outcome.status = Status::Fail;
            outcome.message = message;
        }
    }
    (outcome, hung)
}

/// Restore owner permissions below `folder` (links not followed) and return what still
/// blocks deleting it, as paths relative to `folder` (at most 8).
fn deletable(folder: &Path) -> Vec<String> {
    fixtures::restore_owner_write(folder);
    fixtures::owner_locked(folder)
        .iter()
        .take(8)
        .map(|path| {
            path.strip_prefix(folder)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect()
}
