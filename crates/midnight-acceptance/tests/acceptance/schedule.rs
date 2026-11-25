//! Only private workers share a scheduling phase. The desktop phase starts after
//! all of them have exited, including any plugin processes they launched.
use super::{Isolation, Options, Outcome, Scenario, Status, fixtures::Fixture, run_one};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

pub(super) fn run(
    selected: &[&'static Scenario],
    fixture: &Arc<Fixture>,
    run_dir: &Path,
    options: &Arc<Options>,
    started: Instant,
) -> (Vec<(&'static Scenario, Outcome)>, bool) {
    let stopped = AtomicBool::new(false);
    let execute = |scenario: &'static Scenario, parallel| {
        if stopped.load(Ordering::Acquire) {
            return (
                scenario,
                Outcome::with(Status::NotRun, "an earlier scenario hung"),
            );
        }
        println!(
            "Starting {} ({})",
            scenario.name,
            scenario.isolation.label()
        );
        let start_offset = started.elapsed();
        let (mut outcome, hung) = run_one(scenario, fixture, run_dir, options, parallel);
        outcome.start_offset = start_offset;
        if hung {
            stopped.store(true, Ordering::Release);
        }
        println!(
            "{:<28} {} {:>7.1} s {}",
            scenario.name,
            outcome.status.label(),
            outcome.duration.as_secs_f64(),
            outcome.headline()
        );
        (scenario, outcome)
    };
    let mut safe: Vec<_> = selected
        .iter()
        .copied()
        .filter(|s| s.isolation.effective() != Isolation::Exclusive)
        .collect();
    let jobs = options.jobs.min(safe.len());
    if jobs < 2 {
        let results = selected.iter().map(|s| execute(s, false)).collect();
        return (results, stopped.into_inner());
    }

    println!(
        "Running {} parallel-safe scenarios with {jobs} workers; {} exclusive",
        safe.len(),
        selected.len() - safe.len()
    );
    // Start the scenarios with larger time budgets early; a free worker takes
    // the next scenario without waiting for the others in its original batch.
    safe.sort_by_key(|s| std::cmp::Reverse(s.timeout));
    let cursor = AtomicUsize::new(0);
    let mut outcomes = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    while let Some(&scenario) = safe.get(cursor.fetch_add(1, Ordering::Relaxed)) {
                        done.push(execute(scenario, true));
                    }
                    done
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("acceptance scheduler worker"))
            .map(|(scenario, outcome)| (scenario.name, outcome))
            .collect::<std::collections::HashMap<_, _>>()
    });
    let results = selected
        .iter()
        .map(|&scenario| {
            if scenario.isolation.effective() == Isolation::Exclusive {
                execute(scenario, false)
            } else {
                (
                    scenario,
                    outcomes
                        .remove(scenario.name)
                        .expect("worker returned this scenario"),
                )
            }
        })
        .collect();
    (results, stopped.into_inner())
}
