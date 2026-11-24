//! Journey: a filtered panel with hidden files and a mask selection, copied onto
//! existing names with three different answers; the job's own window, both panels and
//! the Jobs history afterwards.
//!
//! 1. Ctrl+F12 filters the source to `*.txt`; dot-names are shown (the default), so
//!    `.hidden.txt` is listed.
//! 2. Num + selects `*.txt|echo*`: the shown .txt files except echo.txt, the hidden one
//!    included.
//! 3. Ctrl+H hides dot-names in both panels: the hidden file's mark goes, the filter
//!    stays.
//! 4. F5 onto a folder that holds three of the names: each question names its file;
//!    Replace (alpha), Skip (bravo), Copy as (delta).
//! 5. The job ends Completed with one skip and one recovery copy; its window stays and
//!    says so; no notice.
//! 6. On disk: replaced, kept, renamed and recovered exactly; nothing unmarked, hidden or
//!    filtered out was copied; the sources are unchanged.
//! 7. Panels: the new name is listed; the recovery folder is a dot-name, hidden while
//!    dot-names are hidden and listed after Ctrl+H; the source keeps its filter and only
//!    the skipped file stays marked.
//! 8. Jobs lists the job as completed with warnings: 2 files · 1 skipped · 0 errors.
use super::journeys_support::{self as journey, Journey, wait};
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as files, Choice, TransferSpec},
    input, panel,
};
use midnight_core::{CopyOperationState, PaneId};
use midnight_ui::PanelChoices;
use slint::{ComponentHandle, platform::Key};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const STEP: Duration = Duration::from_secs(30);
const RENAMED: &str = "delta (from source).txt";
const SOURCE: [(&str, &str); 7] = [
    ("alpha.txt", "alpha new\n"),
    ("bravo.txt", "bravo new\n"),
    ("delta.txt", "delta new\n"),
    ("echo.txt", "echo new\n"),
    (".hidden.txt", "hidden new\n"),
    ("notes.log", "log\n"),
    ("keep.dat", "dat\n"),
];
const TARGET: [(&str, &str); 4] = [
    ("alpha.txt", "alpha old\n"),
    ("bravo.txt", "bravo old\n"),
    ("delta.txt", "delta old\n"),
    (".hidden.txt", "hidden old\n"),
];

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_filtered_conflicts",
        area: "journeys",
        summary: "A *.txt panel filter, a Num + mask selection and Ctrl+H, then F5 onto existing names with Replace, Skip and Copy as: the job window's outcome, the disk, both panels (recovery folder follows Ctrl+H) and the Jobs history.",
        run: filtered_conflicts,
        timeout: Duration::from_secs(8 * 60),
    }
}

pub fn filtered_conflicts(context: &mut Context) -> Result<(), String> {
    let source = files::scratch(context, "source")?;
    let target = files::scratch(context, "target")?;
    for (name, text) in SOURCE {
        journey::write(&source.join(name), text)?;
    }
    for (name, text) in TARGET {
        journey::write(&target.join(name), text)?;
    }
    // Windows hides by the hidden attribute, not by name.
    for folder in [&source, &target] {
        crate::fixtures::hide(&folder.join(".hidden.txt"))?;
    }
    let mut app = App::start(Config::new(&source, &target));
    let mut journey = Journey::default();
    let ended = steps(&mut app, context, &mut journey, &source, &target);
    files::dismiss_all(&mut app);
    let ended = ended.and(app.shutdown());
    journey.finish(context, ended)
}

fn mask_open(app: &App) -> bool {
    !app.window
        .global::<PanelChoices>()
        .get_mask_token()
        .is_empty()
}

/// Ctrl+F12 in the left panel: Masks `text`, OK.
fn apply_filter(app: &mut App, text: &str) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    wait(app, "the Panel filter dialog", STEP, |app| {
        app.window.get_panel_filter_view().open
    })?;
    files::choose(app, "Filter mode", "Masks", 3)?;
    input::fill(app.slint(), "File filter expression", text)?;
    input::click_button(app.slint(), "OK")?;
    wait(app, "the filter dialog to close", STEP, |app| {
        !app.window.get_panel_filter_view().open
    })
}

/// Num + (as the native key hook delivers it) in the left panel: Select files `mask`.
fn select_by_mask(app: &mut App, mask: &str) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    if !app
        .window
        .invoke_physical_shortcut("NumpadAdd".into(), false, false, false, false)
    {
        return Err("Num + was not taken by the file list".into());
    }
    wait(app, "the Select files dialog", STEP, mask_open)?;
    input::fill(app.slint(), "Selection mask", mask)?;
    input::key(app.slint(), Key::Return);
    wait(app, "the Select files dialog to close", STEP, |app| {
        !mask_open(app)
    })
}

/// The answer for a question about one of the three conflicting names.
fn decide(texts: &[String]) -> Option<Choice> {
    let mentions = |name: &str| texts.iter().any(|text| text.contains(name));
    match (
        mentions("alpha.txt"),
        mentions("bravo.txt"),
        mentions("delta.txt"),
    ) {
        (true, false, false) => Some(Choice::Replace),
        (false, true, false) => Some(Choice::Skip),
        (false, false, true) => Some(Choice::CopyAs(RENAMED.into())),
        _ => None,
    }
}

/// The recovery folders' files in `folder` and their text.
fn recovered(folder: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".midnight-recovery-")
        {
            continue;
        }
        let mut stack = vec![entry.path()];
        while let Some(path) = stack.pop() {
            for child in fs::read_dir(&path).into_iter().flatten().flatten() {
                let child_path: PathBuf = child.path();
                if child_path.is_dir() {
                    stack.push(child_path);
                } else {
                    found.push((
                        child.file_name().to_string_lossy().into_owned(),
                        journey::read(&child_path),
                    ));
                }
            }
        }
    }
    found
}

fn ctrl_h(app: &mut App) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "h");
    app.settle();
    Ok(())
}

fn steps(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    source: &Path,
    target: &Path,
) -> Result<(), String> {
    // 1. The filter.
    apply_filter(app, "*.txt")?;
    journey::wait_shown(
        app,
        LEFT,
        &[
            ".hidden.txt",
            "alpha.txt",
            "bravo.txt",
            "delta.txt",
            "echo.txt",
        ],
        STEP,
    )
    .map_err(|error| format!("Ctrl+F12 *.txt: {error}"))?;
    journey.step("Ctrl+F12 Masks *.txt lists the five .txt files, the dot-name included");

    // 2. The mask selection.
    select_by_mask(app, "*.txt|echo*")?;
    journey::same(
        &journey::marked(app, LEFT),
        &[".hidden.txt", "alpha.txt", "bravo.txt", "delta.txt"],
    )
    .map_err(|error| format!("Num + *.txt|echo* marked the wrong files: {error}"))?;
    journey.step("Num + *.txt|echo* marks the four .txt files other than echo.txt");

    // 3. Hidden files off.
    ctrl_h(app)?;
    wait(app, "Ctrl+H to hide .hidden.txt", STEP, |app| {
        !journey::shown(app, LEFT).contains(".hidden.txt")
    })?;
    journey.check(
        "Ctrl+H keeps the panel filter",
        journey::same(
            &journey::shown(app, LEFT),
            &["alpha.txt", "bravo.txt", "delta.txt", "echo.txt"],
        ),
    );
    journey.check(
        "Ctrl+H hides dot-names in the other panel too",
        if journey::shown(app, RIGHT).contains(".hidden.txt") {
            Err(format!(
                "the right panel lists {:?}",
                journey::shown(app, RIGHT)
            ))
        } else {
            Ok(())
        },
    );
    journey::same(
        &journey::marked(app, LEFT),
        &["alpha.txt", "bravo.txt", "delta.txt"],
    )
    .map_err(|error| format!("Ctrl+H did not drop the hidden file's mark: {error}"))?;
    app.screenshot(
        context,
        "marked-filtered",
        "*.txt filter, dot-names hidden: three files marked",
    );
    journey.step("Ctrl+H hides dot-names in both panels and drops .hidden.txt's mark");

    // 4. F5 with a different answer for each conflict.
    let start = files::start_transfer(app, context, TransferSpec::copy())?;
    let (progress, questions) = journey::answer_job(app, start, Duration::from_secs(120), decide)?;
    journey.check(
        "one question per conflicting file",
        if questions.len() == 3 {
            Ok(())
        } else {
            Err(format!("{} questions: {questions:?}", questions.len()))
        },
    );
    journey.check(
        "the job ends Completed with one skip, no failure and one recovery copy",
        if progress.state == CopyOperationState::Completed
            && progress.skipped == 1
            && progress.failed == 0
            && progress.recovery_count == 1
            && progress.files_completed == 2
        {
            Ok(())
        } else {
            Err(format!(
                "{:?}: {} files, {} skipped, {} failed, {} recovered",
                progress.state,
                progress.files_completed,
                progress.skipped,
                progress.failed,
                progress.recovery_count
            ))
        },
    );
    journey.step("F5 asks once per conflict: Replace alpha, Skip bravo, Copy as delta");

    // 5. The job's window says what happened; no notice repeats it.
    let outcome = [
        "Files copied: 2",
        "1 skipped item was not copied.",
        "1 file was kept in a recovery folder; Copy details shows where.",
    ];
    // A tool window is sized before it is read: an unsized one has no area, so none of
    // its text is in view.
    for window in journey::titled(app, "Copy") {
        crate::app::layout_tool(&window, 760, 560);
    }
    app.settle();
    let shown = wait(app, "the Copy window's outcome", STEP, |app| {
        journey::titled(app, "Copy").iter().any(|window| {
            let text = journey::window_text(window);
            outcome.iter().all(|line| text.contains(line))
        })
    });
    let windows = journey::titled(app, "Copy");
    journey.check(
        "the job window lists the copied, skipped and recovered files",
        shown.map_err(|error| {
            format!(
                "{error}; Copy windows say {:?}",
                windows.iter().map(journey::window_text).collect::<Vec<_>>()
            )
        }),
    );
    journey.check(
        "no notice follows the job whose window shows its outcome",
        match files::shown_notice(app) {
            None => Ok(()),
            Some(notice) => Err(format!("{notice:?}")),
        },
    );
    if let Some(window) = windows.last() {
        // A tool window is sized before it is rendered: an unsized one has no area.
        crate::app::layout_tool(window, 760, 560);
        app.settle();
        app.screenshot_window(context, window, "outcome", "The Copy job's outcome");
        journey.check(
            "the finished job's window closes",
            journey::close_window(app, window, "the Copy job window"),
        );
    }
    journey.step("the Copy window stays with the outcome (2 copied, 1 skipped, 1 recovered)");

    // 6. On disk.
    for (path, wanted) in [
        ("alpha.txt", "alpha new\n"),
        ("bravo.txt", "bravo old\n"),
        ("delta.txt", "delta old\n"),
        (RENAMED, "delta new\n"),
        (".hidden.txt", "hidden old\n"),
    ] {
        journey.check(
            &format!("{path} after the copy"),
            journey::holds(&target.join(path), wanted),
        );
    }
    for path in ["echo.txt", "notes.log", "keep.dat"] {
        journey.check(
            &format!("{path} (not marked) was not copied"),
            journey::absent(&target.join(path)),
        );
    }
    let recovery = recovered(target);
    journey.check(
        "the replaced original is in the recovery folder",
        if recovery.iter().any(|(_, text)| text == "alpha old\n") {
            Ok(())
        } else {
            Err(format!("recovery holds {recovery:?}"))
        },
    );
    for (name, text) in SOURCE {
        journey.check(
            &format!("the source {name} is unchanged"),
            journey::holds(&source.join(name), text),
        );
    }
    journey.step("on disk: replaced, kept, renamed and recovered exactly; nothing else copied");

    // 7. Both panels. Windows hides by the hidden attribute, which the recovery folder
    //    does not get (TRANSFERS.md: an owner-only DACL), so there it is listed while
    //    hidden files are hidden.
    let mut names = vec!["alpha.txt", "bravo.txt", "delta.txt", RENAMED];
    let recovery_folders: Vec<String> = fs::read_dir(target)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(".midnight-recovery-"))
                .collect()
        })
        .unwrap_or_default();
    if cfg!(windows) {
        names.extend(recovery_folders.iter().map(String::as_str));
    }
    journey.check(
        "the target lists the copy's names, dot-names hidden",
        journey::wait_shown(app, RIGHT, &names, STEP),
    );
    journey.check(
        "the source keeps its filter",
        journey::same(
            &journey::shown(app, LEFT),
            &["alpha.txt", "bravo.txt", "delta.txt", "echo.txt"],
        ),
    );
    journey.check(
        "only the skipped file stays marked",
        journey::same(&journey::marked(app, LEFT), &["bravo.txt"]),
    );
    ctrl_h(app)?;
    let listed = wait(app, "Ctrl+H to show the dot-names again", STEP, |app| {
        let right = journey::shown(app, RIGHT);
        right.contains(".hidden.txt")
            && right
                .iter()
                .any(|name| name.starts_with(".midnight-recovery-"))
    });
    journey.check(
        "Ctrl+H lists the recovery folder and .hidden.txt in the target",
        listed.map_err(|error| format!("{error}; it lists {:?}", journey::shown(app, RIGHT))),
    );
    journey.check(
        "the shown hidden source file is not marked again",
        journey::same(&journey::marked(app, LEFT), &["bravo.txt"]),
    );
    app.screenshot(
        context,
        "after-copy",
        "After the copy: dot-names shown again in both panels",
    );
    journey.step("panels: the new name listed, the recovery folder follows Ctrl+H, marks kept");

    // 8. Jobs.
    let jobs = journey::open_jobs(app)?;
    let rows = journey::job_rows(&jobs);
    journey.check(
        "Jobs lists the copy as completed with warnings",
        match journey::job_row(
            &rows,
            "Copy",
            &["Completed with warnings", "2 files · 1 skipped · 0 errors"],
        ) {
            Some(_) => Ok(()),
            None => Err(format!("Jobs lists {rows:?}")),
        },
    );
    crate::app::layout_tool(&jobs, 760, 560);
    app.settle();
    app.screenshot_window(context, &jobs, "jobs", "Jobs after the copy");
    journey::close_window(app, &jobs, "the Jobs window")?;
    journey.step("Jobs lists the copy: Completed with warnings, 2 files · 1 skipped · 0 errors");
    Ok(())
}
