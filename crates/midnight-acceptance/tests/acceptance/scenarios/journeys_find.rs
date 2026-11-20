//! Journey: from a content search to verified copies. Staged with the installed
//! plugins (archives, viewers, comparators).
//!
//! 1. Alt+F7 finds `needle` in `*.txt` files, Search inside archives on: three files
//!    in two folders and one ZIP member.
//! 2. Feed to panel lists the three files; the member is left out and counted.
//! 3. F3 on the first result shows its text; closing the viewer leaves the list as it was.
//! 4. Insert marks the three results; F5 copies them to the other panel (one folder).
//! 5. Backspace leaves the list; Ctrl+Shift+C compares a result with its copy by name
//!    across the panels (identical, also from a subfolder) and two different copies
//!    (different).
//! 6. Ctrl+F8 on the copies: each SHA-256 equals the original's (an independent
//!    digest); Save manifest… and Verify file confirm all three.
use super::{
    find_support::{self as find, Query},
    journeys_support::{self as journey, Journey, wait},
};
use crate::{
    Context,
    app::{App, Config},
    encode::{self, Member},
    files_support::{self as files, TransferSpec},
    input, inspect, panel, tools_support as tools, viewers_stage, viewers_support as support,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CopyOperationState, PaneId};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const STEP: Duration = Duration::from_secs(30);
const OPEN_LIMIT: Duration = Duration::from_secs(60);
pub const TIMEOUT: Duration = Duration::from_secs(12 * 60);
/// The comparator's `identity-state` verdicts.
const IDENTICAL: i32 = 1;
const DIFFERENT: i32 = 3;
/// The three files that match, relative to the searched folder, and their text.
const MATCHES: [(&str, &str); 3] = [
    ("alpha.txt", "Alpha: the needle is here.\n"),
    ("omega.txt", "Omega keeps a needle too, and more words.\n"),
    ("docs/gamma.txt", "Gamma: a needle in a subfolder.\n"),
];

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_find_to_checksums",
        area: "journeys",
        summary: "Find by content inside archives, Feed to panel (member counted), F3 on a result, F5 of the marked results, Ctrl+Shift+C across the panels and between two copies, Ctrl+F8 digests equal to the originals' and a saved manifest verified (installed plugins).",
        run: find_to_checksums,
        timeout: TIMEOUT,
    }
}

pub fn find_to_checksums(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn prepare(work: &Path) -> Result<(PathBuf, PathBuf), String> {
    let project = work.join("project");
    for (path, text) in MATCHES {
        journey::write(&project.join(path), text)?;
    }
    journey::write(&project.join("beta.txt"), "Beta has no match.\n")?;
    journey::write(
        &project.join("docs/delta.md"),
        "Delta has a needle, but it is not a .txt file.\n",
    )?;
    journey::write(
        &project.join("bundle.zip"),
        encode::zip(&[
            Member::directory("inside"),
            Member::file("inside/epsilon.txt", "Epsilon: a needle in the archive.\n"),
            Member::file("inside/zeta.txt", "Zeta has no match.\n"),
        ]),
    )?;
    let out = work.join("out");
    std::fs::create_dir_all(&out).map_err(|error| files::io_error(&out, error))?;
    Ok((project, out))
}

fn run(context: &mut Context) -> Result<(), String> {
    let (project, out) = prepare(&context.work)?;
    let mut config = Config::new(&project, &out);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let mut journey = Journey::default();
    let ended = steps(&mut app, context, &mut journey, &project, &out);
    find::reset(&mut app);
    files::dismiss_all(&mut app);
    let ended = ended.and(app.shutdown());
    journey.finish(context, ended)
}

/// Ctrl+Shift+C on the focused panel; the comparator's verdict, then Escape.
fn compare(app: &mut App, what: &str, wanted: i32) -> Result<(), String> {
    panel::wait_input(app);
    let before = support::plugins(app)?.compared.borrow().len();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let compared = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .cloned()
        .ok_or_else(|| {
            format!(
                "{what}: Ctrl+Shift+C opened no comparator; notice {:?}",
                app.notice()
            )
        })?;
    let window = compared
        .window
        .upgrade()
        .ok_or_else(|| format!("{what}: the comparator closed at once"))?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let deadline = Instant::now() + OPEN_LIMIT;
    let verdict = loop {
        app.settle();
        support::paint_tool_windows(app);
        let state = window.get_identity_state();
        if matches!(state, 1..=3) {
            break state;
        }
        if Instant::now() >= deadline {
            break state;
        }
        app.idle(Duration::from_millis(10));
    };
    let status = window.get_status().to_string();
    input::key(window.window(), Key::Escape);
    let closed = wait(app, "Escape to close the comparator", STEP, |app| {
        support::software_window(app, window.window()).is_none()
    });
    app.slint()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    app.settle();
    if verdict != wanted {
        return Err(format!(
            "{what}: verdict {verdict} (status {status:?}), expected {wanted}"
        ));
    }
    closed
}

/// Mark exactly `names` in `pane` with Insert.
fn mark(app: &mut App, pane: PaneId, names: &[&str]) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    for name in names {
        panel::select_row(app, pane, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    journey::same(&journey::marked(app, pane), names)
}

fn steps(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    project: &Path,
    out: &Path,
) -> Result<(), String> {
    // 1. Find.
    let window = find::open_on(app, LEFT)?;
    let (status, rows) = find::search(
        app,
        &window,
        Query {
            masks: "*.txt",
            content: Some("needle"),
            archives: true,
            ..find::BASE
        },
    )?;
    let found = find::relative(&rows, project);
    journey::same(
        &found,
        &[
            "alpha.txt",
            "omega.txt",
            "docs/gamma.txt",
            "bundle.zip/inside/epsilon.txt",
        ],
    )
    .map_err(|error| format!("Find needle in *.txt inside archives: {error} ({status})"))?;
    journey.check(
        "the search completes",
        if status.starts_with("Completed") {
            Ok(())
        } else {
            Err(status.clone())
        },
    );
    app.screenshot_window(
        context,
        &window,
        "find",
        "needle found in files and a member",
    );
    journey.step("Alt+F7 needle in *.txt with archives: 3 files and 1 ZIP member");

    // 2. Feed to panel.
    tools::click_last(window.window(), "Feed to panel")?;
    wait(app, "the fed list", STEP, |app| {
        find::feed_view(app, LEFT).open
    })?;
    let watched = window.clone();
    wait(app, "Feed to panel to close Find", STEP, move |_| {
        !watched.window().is_visible()
    })?;
    panel::wait_input(app);
    let fed: std::collections::BTreeSet<String> = find::feed_names(app, LEFT).into_iter().collect();
    journey::same(&fed, &["alpha.txt", "omega.txt", "gamma.txt"])
        .map_err(|error| format!("the fed list: {error}"))?;
    let status = find::feed_view(app, LEFT).status.to_string();
    journey.check(
        "the fed list counts the member it leaves out",
        if status.contains("1 archive member") {
            Ok(())
        } else {
            Err(format!("status {status:?}"))
        },
    );
    journey.check(
        "the fed list's count of left-out members is grammatical (finding J-1)",
        if status.contains("1 archive members") {
            Err(format!(
                "status {status:?}: one member is counted in the plural"
            ))
        } else {
            Ok(())
        },
    );
    app.screenshot(context, "fed", "The results fed to the panel");
    journey.step(format!("Feed to panel: 3 files listed; status {status:?}"));

    // 3. F3 on the first result.
    input::key(app.slint(), Key::Home);
    app.settle();
    let first = find::feed_cursor(app, LEFT).ok_or("the fed list has no cursor")?;
    let text = MATCHES
        .iter()
        .find(|(path, _)| Path::new(path).file_name() == Some(std::ffi::OsStr::new(&first)))
        .map(|(_, text)| text.trim_end().to_owned())
        .ok_or_else(|| format!("the first result {first:?} is not a match"))?;
    let before = support::viewers_opened(app);
    input::key(app.slint(), Key::F3);
    let viewing = support::next_viewer(app, before, OPEN_LIMIT)?;
    let shown = viewing.wait(app, "the viewer to show the result", STEP, |window| {
        window.get_accessible_text().contains(&text)
    });
    app.screenshot_window(
        context,
        &viewing.software,
        "viewer",
        "F3 on the first result",
    );
    viewing.close(app)?;
    journey.check("F3 shows the result's text", shown);
    journey.check(
        "the list is unchanged after the viewer closes",
        if find::feed_view(app, LEFT).open && find::feed_cursor(app, LEFT) == Some(first.clone()) {
            Ok(())
        } else {
            Err(format!(
                "open {}, cursor {:?}",
                find::feed_view(app, LEFT).open,
                find::feed_cursor(app, LEFT)
            ))
        },
    );
    journey.step(format!(
        "F3 on {first} showed its text; Escape back to the list"
    ));

    // 4. Mark the results and copy them.
    panel::wait_input(app);
    input::key(app.slint(), Key::Home);
    for _ in 0..fed.len() {
        input::key(app.slint(), Key::Insert);
    }
    app.settle();
    let marked: std::collections::BTreeSet<String> =
        find::feed_marked(app, LEFT).into_iter().collect();
    journey::same(&marked, &["alpha.txt", "omega.txt", "gamma.txt"])
        .map_err(|error| format!("Insert in the fed list: {error}"))?;
    let run = files::transfer(app, context, TransferSpec::copy())?;
    files::completed(&run, "F5 of the marked results")?;
    for (path, text) in MATCHES {
        let name = Path::new(path).file_name().unwrap_or_default();
        journey.check(
            &format!("{path} copied to the other panel"),
            journey::holds(&out.join(name), text),
        );
    }
    journey.check(
        "the other panel lists the copies while the list stays fed",
        journey::wait_shown(app, RIGHT, &["alpha.txt", "omega.txt", "gamma.txt"], STEP).and_then(
            |()| {
                if find::feed_view(app, LEFT).open {
                    Ok(())
                } else {
                    Err("the fed list closed".into())
                }
            },
        ),
    );
    context.note(format!(
        "marked in the fed list after the copy: {:?}",
        find::feed_marked(app, LEFT)
    ));
    journey.step("Insert × 3 and F5: the three results copied to one folder");

    // 5. Compare.
    panel::wait_input(app);
    input::key(app.slint(), Key::Backspace);
    wait(app, "Backspace to leave the list", STEP, |app| {
        !find::feed_view(app, LEFT).open
    })?;
    panel::wait_location(app, LEFT, project);
    panel::select_row(app, LEFT, "alpha.txt")?;
    journey.check(
        "alpha.txt and its copy (by name across the panels) are identical",
        compare(app, "alpha.txt with its copy", IDENTICAL),
    );
    panel::enter(app, LEFT, "docs")?;
    panel::select_row(app, LEFT, "gamma.txt")?;
    journey.check(
        "docs/gamma.txt and its copy are identical",
        compare(app, "docs/gamma.txt with its copy", IDENTICAL),
    );
    panel::leave(app, LEFT)?;
    mark(app, RIGHT, &["alpha.txt", "omega.txt"])?;
    journey.check(
        "two different copies compare as different",
        compare(app, "alpha.txt with omega.txt", DIFFERENT),
    );
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    journey.step("Ctrl+Shift+C: alpha and gamma identical to their copies; alpha ≠ omega");

    // 6. Checksums of the copies.
    mark(app, RIGHT, &["alpha.txt", "omega.txt", "gamma.txt"])?;
    let window = tools::open_tool(
        app,
        "the Checksums window",
        &[Key::Control],
        Key::F8,
        (1000, 720),
    );
    tools::click_last(window.window(), "Calculate")?;
    let watched = window.clone();
    wait(app, "the checksums", STEP, move |_| {
        inspect::labels(watched.window(), AccessibleRole::Text)
            .iter()
            .any(|text| text.starts_with("Completed · 3 calculated"))
    })?;
    let rows = inspect::labels(window.window(), AccessibleRole::ListItem);
    for (path, _) in MATCHES {
        let name = Path::new(path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let digest = journey::sha256(&project.join(path))?;
        journey.check(
            &format!("the copy of {path} has the original's SHA-256"),
            if rows
                .iter()
                .any(|row| row.starts_with(&format!("{name} · ")) && row.contains(&digest))
            {
                Ok(())
            } else {
                Err(format!("no row {name} with {digest}: {rows:?}"))
            },
        );
    }
    let saves = app.manifests.len();
    tools::click_last(window.window(), "Save manifest…")?;
    wait(app, "the manifest to be saved", STEP, move |app| {
        app.manifests.len() > saves
    })?;
    journey.check(
        "Save manifest… writes beside the copies",
        match app.manifests.last() {
            Some(Ok(_)) if out.join("checksums.sha256").is_file() => Ok(()),
            other => Err(format!("{other:?}")),
        },
    );
    tools::click_last(window.window(), "Verify file")?;
    app.settle();
    tools::click_last(window.window(), "Verify")?;
    let watched = window.clone();
    journey.check(
        "Verify file confirms the three copies",
        wait(app, "the verification", STEP, move |_| {
            inspect::labels(watched.window(), AccessibleRole::Text)
                .iter()
                .any(|text| text.starts_with("Completed · 3 verified · 0 mismatched · 0 missing"))
        }),
    );
    app.screenshot_window(
        context,
        &window,
        "checksums",
        "The copies' checksums verified",
    );
    journey::close_window(app, &window, "the Checksums window")?;
    journey.check(
        "no checksum errors",
        if app.checksum_errors.is_empty() {
            Ok(())
        } else {
            Err(format!("{:?}", app.checksum_errors))
        },
    );
    journey.check(
        "no copy failed",
        if app
            .copies
            .iter()
            .all(|job| job.state != CopyOperationState::Failed)
        {
            Ok(())
        } else {
            Err("a copy job failed".into())
        },
    );
    journey.step("Ctrl+F8: the copies' SHA-256 equal the originals'; the saved manifest verifies");
    Ok(())
}
