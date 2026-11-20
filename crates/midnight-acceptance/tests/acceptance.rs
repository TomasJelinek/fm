//! Opt-in end-to-end acceptance suite: the real MainWindow, Presenter and Engine on the
//! native filesystem, driven by synthetic keyboard and pointer input on software-rendered
//! windows, against a generated fixture tree.
//!
//! This target needs the `acceptance` feature and has its own `main` (`harness = false`):
//! scenarios run one at a time, each on its own thread (Slint's platform and windows are
//! per thread), under a watchdog. Run it with `./build.sh --acceptance` or
//! `scripts/acceptance.sh`; see docs/developer/ACCEPTANCE.md.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_lines)]

#[path = "acceptance/app.rs"]
mod app;
#[path = "acceptance/encode.rs"]
mod encode;
#[path = "acceptance/fixtures.rs"]
mod fixtures;
#[path = "acceptance/input.rs"]
mod input;
#[path = "acceptance/inspect.rs"]
mod inspect;
#[path = "acceptance/panel.rs"]
mod panel;
#[path = "acceptance/report.rs"]
mod report;
#[path = "acceptance/review.rs"]
mod review;
#[path = "acceptance/scenarios/mod.rs"]
mod scenarios;

use report::{Outcome, Status};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// What every scenario receives.
pub struct Context {
    /// The shared, read-only fixture tree. Scenarios that change files copy what they
    /// need into [`Context::work`] first.
    pub fixture: Arc<fixtures::Fixture>,
    /// This run's output directory (`_build/acceptance/<run>`).
    pub run_dir: PathBuf,
    /// A private, initially empty scratch folder for this scenario.
    pub work: PathBuf,
    pub options: Arc<Options>,
    /// Screenshots, notes and review flags gathered while the scenario runs.
    pub outcome: Outcome,
}

impl Context {
    pub fn note(&mut self, text: impl Into<String>) {
        self.outcome.notes.push(text.into());
    }
}

pub type Run = fn(&mut Context) -> Result<(), String>;

pub struct Scenario {
    pub name: &'static str,
    pub area: &'static str,
    pub summary: &'static str,
    pub run: Run,
    pub timeout: Duration,
}

pub struct Options {
    pub filters: Vec<String>,
    pub scale: fixtures::Scale,
    pub run_dir: PathBuf,
    pub keep_fixture: bool,
    pub strict_review: bool,
    /// Review every theme instead of the representative subset.
    pub all_themes: bool,
    pub list: bool,
}

fn usage() -> &'static str {
    "Usage: acceptance [FILTER...] [--list] [--scale smoke|full] [--run-dir DIR] \
     [--keep-fixture] [--strict-review] [--all-themes]\n\
     FILTER selects scenarios whose name contains it. Environment equivalents: \
     MIDNIGHT_ACCEPTANCE_FILTER (comma-separated), MIDNIGHT_ACCEPTANCE_SCALE, \
     MIDNIGHT_ACCEPTANCE_RUN_DIR, MIDNIGHT_ACCEPTANCE_KEEP_FIXTURE=1, \
     MIDNIGHT_ACCEPTANCE_STRICT_REVIEW=1, MIDNIGHT_ACCEPTANCE_ALL_THEMES=1."
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

fn options() -> Result<Options, String> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut options = Options {
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
        run_dir: std::env::var_os("MIDNIGHT_ACCEPTANCE_RUN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.join("_build/acceptance").join(report::run_id())),
        keep_fixture: env_flag("MIDNIGHT_ACCEPTANCE_KEEP_FIXTURE"),
        strict_review: env_flag("MIDNIGHT_ACCEPTANCE_STRICT_REVIEW"),
        all_themes: env_flag("MIDNIGHT_ACCEPTANCE_ALL_THEMES"),
        list: false,
    };
    let mut scale = std::env::var("MIDNIGHT_ACCEPTANCE_SCALE").ok();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--list" => options.list = true,
            "--keep-fixture" => options.keep_fixture = true,
            "--strict-review" => options.strict_review = true,
            "--all-themes" => options.all_themes = true,
            "--scale" => scale = Some(arguments.next().ok_or("--scale needs a value")?),
            "--run-dir" => {
                options.run_dir = PathBuf::from(arguments.next().ok_or("--run-dir needs a value")?);
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
    options.scale = match scale.as_deref() {
        None | Some("full") => fixtures::Scale::Full,
        Some("smoke") => fixtures::Scale::Smoke,
        Some(other) => return Err(format!("unknown scale {other}; use smoke or full")),
    };
    Ok(options)
}

fn main() -> std::process::ExitCode {
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
    let selected: Vec<&Scenario> = scenarios::all()
        .iter()
        .filter(|scenario| {
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
                "{:<28} {:<12} {}",
                scenario.name, scenario.area, scenario.summary
            );
        }
        return std::process::ExitCode::SUCCESS;
    }
    if selected.is_empty() {
        eprintln!("no scenario matches {:?}", options.filters);
        return std::process::ExitCode::from(2);
    }
    let started = Instant::now();
    if let Err(error) = std::fs::create_dir_all(&options.run_dir) {
        eprintln!("cannot create {}: {error}", options.run_dir.display());
        return std::process::ExitCode::FAILURE;
    }
    let run_dir = options
        .run_dir
        .canonicalize()
        .unwrap_or_else(|_| options.run_dir.clone());
    report::prune_old_runs(&run_dir);
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
    let mut results: Vec<(&Scenario, Outcome)> = Vec::new();
    let mut abandoned = false;
    for scenario in &selected {
        if abandoned {
            results.push((
                scenario,
                Outcome::with(
                    Status::NotRun,
                    "an earlier scenario hung; the process stopped",
                ),
            ));
            continue;
        }
        print!("{:<28} ", scenario.name);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let outcome = run_one(scenario, &fixture, &run_dir, &options);
        println!(
            "{} {:>7.1} s {}",
            outcome.status.label(),
            outcome.duration.as_secs_f64(),
            outcome.headline()
        );
        if outcome.status == Status::Timeout {
            abandoned = true;
        }
        results.push((scenario, outcome));
    }
    let failed = results
        .iter()
        .filter(|(_, outcome)| outcome.status.is_failure(options.strict_review))
        .count();
    let index = report::write(&run_dir, &fixture, &results, started.elapsed(), &options);
    report::print_summary(&results, started.elapsed(), &index, options.strict_review);
    if !options.keep_fixture && failed == 0 {
        let _ = fixtures::remove_tree(&fixture_root);
        let _ = fixtures::remove_tree(&run_dir.join("work"));
    }
    if abandoned {
        // A hung scenario thread cannot be joined; leave without unwinding it.
        std::process::exit(1);
    }
    if failed == 0 {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

fn run_one(
    scenario: &Scenario,
    fixture: &Arc<fixtures::Fixture>,
    run_dir: &Path,
    options: &Arc<Options>,
) -> Outcome {
    let work = run_dir.join("work").join(scenario.name);
    let _ = fixtures::remove_tree(&work);
    if let Err(error) = std::fs::create_dir_all(&work) {
        return Outcome::with(
            Status::Fail,
            format!("cannot create {}: {error}", work.display()),
        );
    }
    let context = Context {
        fixture: fixture.clone(),
        run_dir: run_dir.to_path_buf(),
        work,
        options: options.clone(),
        outcome: Outcome::default(),
    };
    let (sender, receiver) = mpsc::channel();
    let run = scenario.run;
    let started = Instant::now();
    let spawned = std::thread::Builder::new()
        .name(scenario.name.to_owned())
        .stack_size(16 << 20)
        .spawn(move || {
            let mut context = context;
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut context)));
            let mut outcome = std::mem::take(&mut context.outcome);
            match result {
                Ok(Ok(())) => {
                    if outcome.status == Status::Pass && !outcome.flags.is_empty() {
                        outcome.status = Status::Warn;
                    }
                }
                Ok(Err(message)) => {
                    outcome.status = if message.starts_with("SKIP:") {
                        Status::Skip
                    } else {
                        Status::Fail
                    };
                    outcome.message = message.trim_start_matches("SKIP:").trim().to_owned();
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
    let mut outcome = match spawned {
        Err(error) => Outcome::with(
            Status::Fail,
            format!("cannot start the scenario thread: {error}"),
        ),
        Ok(handle) => match receiver.recv_timeout(scenario.timeout) {
            Ok(outcome) => {
                let _ = handle.join();
                outcome
            }
            Err(_) => Outcome::with(
                Status::Timeout,
                format!("no result within {:?}", scenario.timeout),
            ),
        },
    };
    outcome.duration = started.elapsed();
    outcome
}
