//! Journey: work goes on while a copy is paused. Two copies without "Wait for earlier
//! transfers", the first paused as soon as it copies, and a permanent delete
//! in a third folder meanwhile; Jobs shows all three; Resume finishes the first and then
//! the delete; every result is exact on disk and no notice appears.
//!
//! 1. F5 of `many/` (Wait off) to its own folder, paused from its window.
//! 2. F5 of `small/` (Wait off) to another folder: it completes beside the paused copy.
//! 3. Shift+F8 of `doomed/` (a tree) in the same panel: no question; it waits queued and
//!    deletes nothing while the copy stays paused. A Delete holds the whole namespace,
//!    and every transfer keeps its namespace share until it ends, paused or not
//!    (`docs/developer/operation-progress.md`, "Admission policy and waits").
//! 4. Jobs lists the paused copy, the completed copy and the queued delete.
//! 5. Resume: the first copy completes; its window closes; then the delete completes;
//!    both copies equal their sources, the sources are unchanged, `doomed/` is gone; no
//!    notice.
use super::journeys_support::{self as journey, Journey, wait};
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as files, DeleteEnd, TransferSpec, compare_trees, tree_facts},
    input, panel,
    scenarios::transfers::copy_tree,
    tools_support as tools,
};
use midnight_core::{CopyOperationState, OperationState, PaneId};
use slint::platform::{Key, WindowAdapter};
use std::{path::Path, time::Duration};

const LEFT: PaneId = PaneId::Left;

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_concurrent_jobs",
        isolation: crate::Isolation::Parallel,
        area: "journeys",
        summary: "Two copies with Wait for earlier transfers off, the first paused, and a permanent delete in a third folder meanwhile; Jobs lists all three; Resume; every result exact on disk, no notice.",
        run: concurrent_jobs,
        timeout: Duration::from_secs(15 * 60),
    }
}

pub fn concurrent_jobs(context: &mut Context) -> Result<(), String> {
    let root = files::scratch(context, "src")?;
    copy_tree(&context.fixture.path("many"), &root.join("many"))?;
    for index in 0..3 {
        journey::write(
            &root.join(format!("small/small-{index}.txt")),
            format!("small {index}\n"),
        )?;
    }
    for level in 0..4 {
        let folder = (0..=level).fold(root.join("doomed"), |path, depth| {
            path.join(format!("d{depth}"))
        });
        for index in 0..8 {
            journey::write(
                &folder.join(format!("f{index}.bin")),
                vec![level as u8; 512 * (index + 1)],
            )?;
        }
    }
    let first = files::scratch(context, "to-first")?;
    let second = files::scratch(context, "to-second")?;
    let mut app = App::start(Config::new(&root, &first));
    let mut journey = Journey::default();
    let ended = steps(&mut app, context, &mut journey, &root, &first, &second);
    files::dismiss_all(&mut app);
    for window in journey::titled(&app, "Copy") {
        let _ = tools::click_last(window.window(), "Resume");
    }
    let ended = ended.and(app.shutdown());
    journey.finish(context, ended)
}

/// Shift+F8 of the focused row, confirmed with Enter. Returns where the delete's reports
/// start in `app.operations` once it reported itself queued, ended or asked, and 3 s more
/// passed for whatever it does next.
fn delete_queued(app: &mut App) -> Result<usize, String> {
    panel::wait_input(app);
    let start = app.operations.len();
    input::chord(app.slint(), &[Key::Shift], Key::F8);
    wait(
        app,
        "the permanent delete confirmation",
        Duration::from_secs(20),
        |app| !app.window.get_operation_token().is_empty(),
    )?;
    if !app.window.get_operation_dangerous() {
        return Err("Shift+F8 did not ask for a permanent delete".into());
    }
    input::key(app.slint(), Key::Return);
    wait(
        app,
        "the delete to be queued, end or ask",
        Duration::from_secs(30),
        move |app| {
            app.operations[start..].iter().any(|progress| {
                progress.state == OperationState::Queued || progress.state.is_terminal()
            }) || files::delete_question(app).is_some()
        },
    )?;
    app.idle(Duration::from_secs(3));
    Ok(start)
}

/// Whether the delete whose reports start at `start` completes within `limit`.
fn delete_completed(app: &mut App, start: usize, limit: Duration) -> Result<(), String> {
    wait(app, "the delete to end or ask", limit, move |app| {
        app.operations[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
            || files::delete_question(app).is_some()
    })?;
    match files::wait_delete(app, start) {
        DeleteEnd::Done(progress) => files::operation_ok(&progress, "the delete"),
        DeleteEnd::Asked(window) => {
            let asked = format!("it asked: {}", files::describe_window(&window));
            files::dismiss_all(app);
            Err(asked)
        }
    }
}

fn steps(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    root: &Path,
    first: &Path,
    second: &Path,
) -> Result<(), String> {
    let many = tree_facts(&root.join("many"))?;
    let small = tree_facts(&root.join("small"))?;

    // 1. The first copy, paused.
    let before = journey::titled(app, "Copy").len();
    panel::select_row(app, LEFT, "many")?;
    let one = files::start_transfer(
        app,
        context,
        TransferSpec {
            destination: Some(first),
            wait: Some(false),
            ..TransferSpec::copy()
        },
    )?;
    let window = journey::new_window(app, "Copy", before)?;
    journey::pause_job(app, &window, one, "the copy of many/")?;
    let held = files::job(app, one).map(|job| job.files_completed);
    journey.step(format!(
        "F5 of many/ (Wait off) paused after {held:?} files"
    ));

    // 2. The second copy, not waiting.
    panel::select_row(app, LEFT, "small")?;
    let two = files::start_transfer(
        app,
        context,
        TransferSpec {
            destination: Some(second),
            wait: Some(false),
            ..TransferSpec::copy()
        },
    )?;
    let ran = wait(
        app,
        "the second copy to complete beside the paused one",
        Duration::from_secs(20),
        move |app| files::job(app, two).is_some_and(|job| job.state.is_terminal()),
    );
    let second_done =
        files::job(app, two).is_some_and(|job| job.state == CopyOperationState::Completed);
    journey.check(
        "a copy that does not wait completes beside a paused one",
        if second_done {
            Ok(())
        } else {
            Err(format!(
                "{}; it is {:?}",
                ran.err().unwrap_or_default(),
                files::job(app, two).map(|job| (job.state, job.wait_reason))
            ))
        },
    );
    journey.step(format!(
        "F5 of small/ (Wait off) beside the paused copy: completed {second_done}"
    ));

    // 3. A permanent delete meanwhile: it holds the whole namespace, of which the paused
    //    copy keeps a share, so it waits queued until that copy ends.
    panel::select_row(app, LEFT, "doomed")?;
    let deleting = match delete_queued(app) {
        Err(error) => {
            journey.check("Shift+F8 of doomed/ while a copy is paused", Err(error));
            None
        }
        Ok(_) if files::delete_question(app).is_some() => {
            let window = files::delete_question(app).expect("a question");
            journey.check(
                "Shift+F8 of doomed/ while a copy is paused",
                Err(format!("it asked: {}", files::describe_window(&window))),
            );
            files::dismiss_all(app);
            None
        }
        Ok(start) => {
            let latest = app.operations[start..].last().cloned();
            journey.check(
                "Shift+F8 of doomed/ waits queued while a copy is paused",
                match latest {
                    Some(progress) if progress.state == OperationState::Queued => Ok(()),
                    other => Err(format!(
                        "it is {:?}",
                        other.map(|progress| (
                            progress.state,
                            progress.failed,
                            progress.message.clone()
                        ))
                    )),
                },
            );
            journey.check(
                "the queued delete leaves doomed/ on the disk",
                if root.join("doomed").is_dir() {
                    Ok(())
                } else {
                    Err("doomed/ is gone while the delete is queued".into())
                },
            );
            Some(start)
        }
    };
    journey.check(
        "the first copy stays paused meanwhile",
        match files::job(app, one) {
            Some(job)
                if job.paused && !job.state.is_terminal() && Some(job.files_completed) == held =>
            {
                Ok(())
            }
            other => Err(format!(
                "{:?}",
                other.map(|job| (job.state, job.paused, job.files_completed))
            )),
        },
    );
    journey.step("Shift+F8 of doomed/ waits queued while the copy stays paused");

    // 4. Jobs lists all three.
    let jobs = journey::open_jobs(app)?;
    let rows = journey::job_rows(&jobs);
    let paused = rows
        .iter()
        .any(|(_, texts)| texts.iter().any(|text| text.starts_with("Paused")));
    journey.check(
        "Jobs shows the paused copy",
        if paused {
            Ok(())
        } else {
            Err(format!("Jobs lists {rows:?}"))
        },
    );
    journey.check(
        "Jobs shows the queued delete",
        match journey::job_row(&rows, "Permanently delete", &["Queued"]) {
            Some(_) => Ok(()),
            None => Err(format!("Jobs lists {rows:?}")),
        },
    );
    if second_done {
        journey.check(
            "Jobs shows the completed copy",
            match journey::job_row(&rows, "Copy", &["Completed"]) {
                Some(_) => Ok(()),
                None => Err(format!("Jobs lists {rows:?}")),
            },
        );
    }
    journey.check(
        "Jobs lists three jobs",
        if rows.len() == 3 {
            Ok(())
        } else {
            Err(format!("{} rows: {rows:?}", rows.len()))
        },
    );
    app.screenshot_window(context, &jobs, "jobs", "Jobs: paused copy, copy and delete");
    journey::close_window(app, &jobs, "the Jobs window")?;
    journey.step(format!("Jobs lists {} jobs", rows.len()));

    // 5. Resume and finish.
    tools::click_last(window.window(), "Resume")?;
    wait(
        app,
        "the first copy to end after Resume",
        Duration::from_secs(300),
        move |app| files::job(app, one).is_some_and(|job| job.state.is_terminal()),
    )?;
    wait(
        app,
        "the second copy to end",
        Duration::from_secs(120),
        move |app| files::job(app, two).is_some_and(|job| job.state.is_terminal()),
    )?;
    for (start, what) in [(one, "the copy of many/"), (two, "the copy of small/")] {
        journey.check(
            &format!("{what} completes"),
            match files::job(app, start) {
                Some(job) if job.state == CopyOperationState::Completed && job.failed == 0 => {
                    Ok(())
                }
                other => Err(format!("{:?}", other.map(|job| (job.state, job.failed)))),
            },
        );
    }
    // Then the delete runs: no transfer holds a namespace share any more.
    if let Some(start) = deleting {
        journey.check(
            "Shift+F8 of doomed/ completes once the copies ended",
            delete_completed(app, start, Duration::from_secs(120)),
        );
    }
    journey.check(
        "doomed/ is gone from the disk",
        journey::absent(&root.join("doomed")),
    );
    journey.check(
        "doomed/ is gone from the panel",
        wait(
            app,
            "doomed to leave the panel",
            Duration::from_secs(10),
            |app| !journey::shown(app, LEFT).contains("doomed"),
        ),
    );
    let watched = window.clone();
    journey.check(
        "the completed copy's window closes",
        wait(
            app,
            "the window to close",
            Duration::from_secs(20),
            move |_| !watched.window().is_visible(),
        ),
    );
    for (name, facts, target) in [("many", &many, first), ("small", &small, second)] {
        let copied = tree_facts(&target.join(name))?;
        let differences = compare_trees(facts, &copied, false);
        journey.check(
            &format!("{name}/ is copied whole"),
            if differences.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{:?}",
                    differences.iter().take(5).collect::<Vec<_>>()
                ))
            },
        );
        let source = compare_trees(facts, &tree_facts(&root.join(name))?, true);
        journey.check(
            &format!("the source {name}/ is unchanged"),
            if source.is_empty() {
                Ok(())
            } else {
                Err(format!("{:?}", source.iter().take(5).collect::<Vec<_>>()))
            },
        );
    }
    journey.check(
        "no notice after the jobs",
        match files::shown_notice(app) {
            None => Ok(()),
            Some(notice) => Err(format!("{notice:?}")),
        },
    );
    journey.check(
        "the operations reported no failure",
        if app
            .operations
            .iter()
            .any(|progress| progress.state == OperationState::Failed)
        {
            Err("a file operation failed".into())
        } else {
            Ok(())
        },
    );
    app.screenshot(context, "done", "All three jobs done");
    journey.step("Resume: both copies complete and equal their sources; doomed/ stays gone");
    Ok(())
}
