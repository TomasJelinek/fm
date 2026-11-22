//! Compare directories (Ctrl+F10): every option of the dialog on identical,
//! asymmetric, nested and unreadable trees, Compare contents and Include subfolders
//! included. Compare closes the dialog at once, the differences become each panel's
//! selection and no result message appears; invalid options are reported in the
//! dialog, which stays open. A long content comparison (two 24 GiB sparse files) shows
//! its progress on the status line, refuses Create folder, Rename, New file, Delete,
//! Copy, Paste and Show hidden files with the reason while it runs, shows its progress
//! and Cancel in the reopened dialog, and Cancel ends it without selecting anything; the
//! next comparison runs normally; the toolbar's button opens the dialog, and Escape in it
//! cancels a running comparison, selecting nothing.
use crate::{
    Context,
    app::{App, Config},
    input, inspect, panel, tools_support as tools,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::{Key, WindowAdapter};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// The base modification time of every generated file (2023-11-14).
fn at(offset: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000 + offset)
}

#[derive(Clone, Copy)]
struct Options {
    size: bool,
    time: bool,
    tolerance: &'static str,
    exact_case: bool,
    directories: bool,
    contents: bool,
    subfolders: bool,
}

const DEFAULTS: Options = Options {
    size: true,
    time: true,
    tolerance: "2",
    exact_case: true,
    directories: false,
    contents: false,
    subfolders: false,
};

/// One side of a pair: (name, bytes, time offset).
type Side = (&'static str, usize, u64);

/// The asymmetric tree: same-named pairs that differ in one way each, entries on one
/// side only, a case-only name difference and folders.
fn asymmetric(root: &Path) -> Result<(), String> {
    let pairs: [(Side, Side); 8] = [
        (("same.txt", 10, 0), ("same.txt", 10, 0)),
        (("bigger-left.dat", 20, 0), ("bigger-left.dat", 10, 0)),
        (("bigger-right.dat", 10, 0), ("bigger-right.dat", 20, 0)),
        (("newer-left.txt", 10, 100), ("newer-left.txt", 10, 0)),
        (("newer-right.txt", 10, 0), ("newer-right.txt", 10, 100)),
        // Inside the default two-second tolerance, and exactly at it.
        (
            ("within-tolerance.txt", 10, 1),
            ("within-tolerance.txt", 10, 0),
        ),
        (("at-tolerance.txt", 10, 2), ("at-tolerance.txt", 10, 0)),
        // Only the letter case differs.
        (("README.TXT", 10, 0), ("readme.txt", 10, 0)),
    ];
    for ((left, left_size, left_time), (right, right_size, right_time)) in pairs {
        tools::write_at(
            &root.join("left").join(left),
            &vec![b'l'; left_size],
            at(left_time),
        )?;
        tools::write_at(
            &root.join("right").join(right),
            &vec![b'r'; right_size],
            at(right_time),
        )?;
    }
    tools::write_at(&root.join("left/only-left.txt"), b"left", at(0))?;
    tools::write_at(&root.join("right/only-right.txt"), b"right", at(0))?;
    // Folders: one on both sides (different contents, compared only inside), one on
    // each side alone.
    tools::write_at(&root.join("left/dir-both/inner.txt"), b"12345", at(0))?;
    tools::write_at(&root.join("right/dir-both/inner.txt"), b"1234567", at(0))?;
    tools::write_at(&root.join("left/dir-both/deeper/left.txt"), b"x", at(0))?;
    tools::write_at(&root.join("right/dir-both/deeper/right.txt"), b"y", at(0))?;
    tools::write_at(&root.join("left/dir-both/shared.txt"), b"same", at(5))?;
    tools::write_at(&root.join("right/dir-both/shared.txt"), b"same", at(5))?;
    tools::write_at(&root.join("left/dir-left-only/file.txt"), b"x", at(0))?;
    tools::write_at(&root.join("right/dir-right-only/file.txt"), b"x", at(0))?;
    Ok(())
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// The names the panel shows selected (the parent entry never counts).
fn selected(app: &App, pane: PaneId) -> BTreeSet<String> {
    app.rows(pane)
        .iter()
        .filter(|row| row.selected && !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect()
}

fn open_dialog(app: &mut App) -> Result<(), String> {
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], Key::F10);
    app.until("the Compare directories dialog", |app| {
        app.window.get_comparison_view().open
    });
    let main = app.main.clone();
    if !tools::has(main.window(), AccessibleRole::Button, "Compare") {
        return Err("the dialog shows no Compare button".into());
    }
    Ok(())
}

fn set_options(app: &mut App, options: Options) -> Result<(), String> {
    let main = app.main.clone();
    let window = main.window();
    // Include subfolders disables Select unmatched directories: clear it first.
    tools::set_check(app, window, "Include subfolders", false)?;
    for (label, wanted) in [
        ("Compare file size", options.size),
        ("Compare modification time", options.time),
        ("Compare contents", options.contents),
        ("Match names with exact case", options.exact_case),
        ("Select unmatched directories", options.directories),
        ("Include subfolders", options.subfolders),
    ] {
        tools::set_check(app, window, label, wanted)?;
    }
    input::fill(window, "Time tolerance in seconds", options.tolerance)?;
    app.settle();
    Ok(())
}

/// Click the dialog's Compare button (the popup's, not the function bar's).
fn click_compare(app: &App) -> Result<(), String> {
    let main = app.main.clone();
    let button = tools::control(main.window(), AccessibleRole::Button, "Compare")?;
    if inspect::roots(main.window()).len() < 2 {
        return Err("the Compare dialog is not a popup".into());
    }
    input::click_at(main.window(), inspect::center(&button));
    Ok(())
}

/// Compare with `options` and return both panels' selections. Verifies that the
/// dialog closed at the click, before the result arrived, and that no notice or
/// other window reports the result.
fn compare(app: &mut App, options: Options) -> Result<[BTreeSet<String>; 2], String> {
    open_dialog(app)?;
    set_options(app, options)?;
    let tools_before = app.tool_windows().len();
    click_compare(app)?;
    // The callback publishes the closed dialog synchronously, while the Engine works.
    let view = app.window.get_comparison_view();
    if view.open {
        return Err(format!(
            "the dialog stayed open after Compare (error {:?})",
            view.error
        ));
    }
    // The result: a summary in the dialog's state, or a "Compared: …" footer status
    // line (newer builds), or an error.
    app.until("the comparison result", |app| {
        let view = app.window.get_comparison_view();
        !view.busy
            && (!view.summary.is_empty()
                || !view.error.is_empty()
                || footer_status(app).is_some()
                || tools::shown_notice(app).is_some())
    });
    let view = app.window.get_comparison_view();
    if !view.error.is_empty() {
        return Err(format!("the comparison failed: {}", view.error));
    }
    if let Some(notice) = tools::shown_notice(app) {
        return Err(format!("a result notice appeared: {notice:?}"));
    }
    if app.tool_windows().len() != tools_before || inspect::roots(app.slint()).len() > 1 {
        return Err(format!(
            "a window or popup appeared after the comparison: {:?}",
            tools::tool_titles(app)
        ));
    }
    let both = [selected(app, PaneId::Left), selected(app, PaneId::Right)];
    // Where the footer reports the result, its counts are the panels' selections.
    if let Some(status) = footer_status(app) {
        let wanted = if both[0].is_empty() && both[1].is_empty() {
            "Compared: no differences for the chosen criteria".to_owned()
        } else {
            format!(
                "Compared: {} selected in the left panel, {} in the right",
                both[0].len(),
                both[1].len()
            )
        };
        if !status.starts_with(&wanted) {
            return Err(format!(
                "the footer reads {status:?}; the panels show {} and {} selected",
                both[0].len(),
                both[1].len()
            ));
        }
    }
    Ok(both)
}

/// The footer's comparison status, when the build reports it there.
fn footer_status(app: &App) -> Option<String> {
    let status = app.window.get_desktop_status().to_string();
    if status.starts_with("Compared:") {
        return Some(status);
    }
    inspect::visible_texts(app.slint())
        .into_iter()
        .find(|text| text.starts_with("Compared:"))
}

fn expect(
    case: &str,
    actual: &[BTreeSet<String>; 2],
    left: &[&str],
    right: &[&str],
) -> Result<(), String> {
    let wanted = [names(left), names(right)];
    if *actual != wanted {
        return Err(format!(
            "{case}: selected left {:?} right {:?}; expected left {:?} right {:?}",
            actual[0], actual[1], wanted[0], wanted[1]
        ));
    }
    Ok(())
}

/// Reopen the dialog, read its summary, close it.
fn summary(app: &mut App, context: &mut Context, shot: Option<&str>) -> Result<String, String> {
    open_dialog(app)?;
    let summary = app.window.get_comparison_view().summary.to_string();
    let main = app.main.clone();
    if !summary.is_empty() && !inspect::visible_texts(main.window()).contains(&summary) {
        return Err(format!("the dialog does not show its summary {summary:?}"));
    }
    if let Some(name) = shot {
        app.screenshot(context, name, "The reopened dialog shows the last summary");
    }
    input::key(app.slint(), Key::Escape);
    app.until("the Compare dialog to close", |app| {
        !app.window.get_comparison_view().open && !app.window.get_embedded_modal_open()
    });
    Ok(summary)
}

pub fn compare_directories(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("compare");

    // Identical trees: nothing differs, under every option.
    for side in ["left", "right"] {
        for (name, size, offset) in [("a.txt", 5, 0), ("b.bin", 4096, 7), ("c.md", 0, 99)] {
            tools::write_at(
                &root.join("identical").join(side).join(name),
                &vec![b'i'; size],
                at(offset),
            )?;
        }
        fs::create_dir_all(root.join("identical").join(side).join("folder"))
            .map_err(|error| error.to_string())?;
    }
    asymmetric(&root.join("asymmetric"))?;

    let mut app = App::start(Config::new(
        root.join("identical/left"),
        root.join("identical/right"),
    ));
    let both = compare(&mut app, DEFAULTS)?;
    expect("identical trees", &both, &[], &[])?;
    let both = compare(
        &mut app,
        Options {
            directories: true,
            exact_case: false,
            tolerance: "0",
            ..DEFAULTS
        },
    )?;
    expect("identical trees, every option", &both, &[], &[])?;
    let text = summary(&mut app, context, Some("identical-summary"))?;
    // Builds that report on the footer instead may leave the summary empty.
    if !text.is_empty() && text != "Displayed metadata matches for the chosen criteria." {
        return Err(format!("identical trees summarised as {text:?}"));
    }
    // Compare contents reads the three same-sized pairs (the empty one too) and
    // Include subfolders walks the matched folder: identical bytes, still no difference.
    let both = compare(
        &mut app,
        Options {
            contents: true,
            subfolders: true,
            ..DEFAULTS
        },
    )?;
    expect("identical trees, contents and subfolders", &both, &[], &[])?;
    let status = footer_status(&app).unwrap_or_default();
    if !status.contains("3 file pairs read") {
        return Err(format!(
            "identical contents: the footer reads {status:?}, expected 3 file pairs read"
        ));
    }
    app.shutdown()?;

    let left = root.join("asymmetric/left");
    let right = root.join("asymmetric/right");
    let mut app = App::start(Config::new(&left, &right));
    // Size and time with the default two-second tolerance and exact case.
    let both = compare(&mut app, DEFAULTS)?;
    app.screenshot(
        context,
        "asymmetric-default",
        "Differences selected in both panels (size, time, exact case)",
    );
    expect(
        "defaults",
        &both,
        &[
            "only-left.txt",
            "bigger-left.dat",
            "newer-left.txt",
            "README.TXT",
        ],
        &[
            "only-right.txt",
            "bigger-right.dat",
            "newer-right.txt",
            "readme.txt",
        ],
    )?;
    let status = app.window.get_left_status().to_string();
    if !status.contains("4 selected") {
        return Err(format!("the left status reads {status:?}"));
    }
    let text = summary(&mut app, context, Some("asymmetric-summary"))?;
    if !text.is_empty()
        && !text.starts_with("Found 4 differences in the first directory and 4 in the second")
    {
        return Err(format!("the summary reads {text:?}"));
    }
    // Size only: newer files are no longer differences.
    let both = compare(
        &mut app,
        Options {
            time: false,
            ..DEFAULTS
        },
    )?;
    expect(
        "size only",
        &both,
        &["only-left.txt", "bigger-left.dat", "README.TXT"],
        &["only-right.txt", "bigger-right.dat", "readme.txt"],
    )?;
    // Time only: larger files are no longer differences.
    let both = compare(
        &mut app,
        Options {
            size: false,
            ..DEFAULTS
        },
    )?;
    expect(
        "time only",
        &both,
        &["only-left.txt", "newer-left.txt", "README.TXT"],
        &["only-right.txt", "newer-right.txt", "readme.txt"],
    )?;
    // Neither: only names without a match.
    let both = compare(
        &mut app,
        Options {
            size: false,
            time: false,
            ..DEFAULTS
        },
    )?;
    expect(
        "names only",
        &both,
        &["only-left.txt", "README.TXT"],
        &["only-right.txt", "readme.txt"],
    )?;
    // Tolerance 0: one and two seconds newer both count.
    let both = compare(
        &mut app,
        Options {
            tolerance: "0",
            ..DEFAULTS
        },
    )?;
    expect(
        "tolerance 0",
        &both,
        &[
            "only-left.txt",
            "bigger-left.dat",
            "newer-left.txt",
            "within-tolerance.txt",
            "at-tolerance.txt",
            "README.TXT",
        ],
        &[
            "only-right.txt",
            "bigger-right.dat",
            "newer-right.txt",
            "readme.txt",
        ],
    )?;
    // Tolerance 1: a difference equal to the tolerance counts as equal.
    let both = compare(
        &mut app,
        Options {
            tolerance: "1",
            ..DEFAULTS
        },
    )?;
    expect(
        "tolerance 1",
        &both,
        &[
            "only-left.txt",
            "bigger-left.dat",
            "newer-left.txt",
            "at-tolerance.txt",
            "README.TXT",
        ],
        &[
            "only-right.txt",
            "bigger-right.dat",
            "newer-right.txt",
            "readme.txt",
        ],
    )?;
    // Case folding pairs README.TXT with readme.txt, which then match.
    let both = compare(
        &mut app,
        Options {
            exact_case: false,
            ..DEFAULTS
        },
    )?;
    expect(
        "case folding",
        &both,
        &["only-left.txt", "bigger-left.dat", "newer-left.txt"],
        &["only-right.txt", "bigger-right.dat", "newer-right.txt"],
    )?;
    // Unmatched directories: folders without a partner join the selection; the
    // matched dir-both is never selected (no recursion).
    let both = compare(
        &mut app,
        Options {
            directories: true,
            ..DEFAULTS
        },
    )?;
    app.screenshot(
        context,
        "asymmetric-directories",
        "Select unmatched directories adds the one-sided folders",
    );
    expect(
        "unmatched directories",
        &both,
        &[
            "only-left.txt",
            "bigger-left.dat",
            "newer-left.txt",
            "README.TXT",
            "dir-left-only",
        ],
        &[
            "only-right.txt",
            "bigger-right.dat",
            "newer-right.txt",
            "readme.txt",
            "dir-right-only",
        ],
    )?;

    // Compare contents: the same-sized pairs that size and time call equal are read
    // byte by byte. Their bytes differ ('l' against 'r'), so both sides are selected;
    // the pairs that size or time already separate are not read.
    let default_left = [
        "only-left.txt",
        "bigger-left.dat",
        "newer-left.txt",
        "README.TXT",
    ];
    let default_right = [
        "only-right.txt",
        "bigger-right.dat",
        "newer-right.txt",
        "readme.txt",
    ];
    let read = ["same.txt", "within-tolerance.txt", "at-tolerance.txt"];
    let with = |base: &[&'static str], extra: &[&'static str]| -> Vec<&'static str> {
        base.iter().chain(extra).copied().collect()
    };
    let both = compare(
        &mut app,
        Options {
            contents: true,
            ..DEFAULTS
        },
    )?;
    app.screenshot(
        context,
        "asymmetric-contents",
        "Compare contents also selects same-sized files whose bytes differ",
    );
    expect(
        "contents",
        &both,
        &with(&default_left, &read),
        &with(&default_right, &read),
    )?;
    let status = footer_status(&app).unwrap_or_default();
    if !status.contains("3 with different contents") || !status.contains("3 file pairs read") {
        return Err(format!(
            "contents: the footer reads {status:?}, expected 3 different of 3 read"
        ));
    }
    // Include subfolders: dir-both differs inside on both sides (a larger file on the
    // right, a file on each side alone two levels down); one-sided folders count.
    let both = compare(
        &mut app,
        Options {
            subfolders: true,
            ..DEFAULTS
        },
    )?;
    app.screenshot(
        context,
        "asymmetric-subfolders",
        "Include subfolders selects the folders that contain differences",
    );
    expect(
        "subfolders",
        &both,
        &with(&default_left, &["dir-both", "dir-left-only"]),
        &with(&default_right, &["dir-both", "dir-right-only"]),
    )?;
    let status = footer_status(&app).unwrap_or_default();
    if !status.contains("1 folder with differences") {
        return Err(format!(
            "subfolders: the footer reads {status:?}, expected 1 folder with differences"
        ));
    }
    let text = summary(&mut app, context, Some("subfolders-summary"))?;
    if !text.is_empty() && !text.contains("Folders with differences: 1.") {
        return Err(format!("subfolders: the summary reads {text:?}"));
    }
    // Both: dir-both/shared.txt has equal bytes and stays a match inside.
    let both = compare(
        &mut app,
        Options {
            contents: true,
            subfolders: true,
            ..DEFAULTS
        },
    )?;
    expect(
        "contents and subfolders",
        &both,
        &with(&with(&default_left, &read), &["dir-both", "dir-left-only"]),
        &with(
            &with(&default_right, &read),
            &["dir-both", "dir-right-only"],
        ),
    )?;

    // Invalid tolerances are reported in the dialog, which stays open; nothing runs.
    for (text, message) in [
        ("abc", "Enter a whole number of seconds from 0 to 86400."),
        (
            "86401",
            "Time tolerance must be between 0 and 86400 seconds.",
        ),
    ] {
        open_dialog(&mut app)?;
        set_options(
            &mut app,
            Options {
                tolerance: text,
                ..DEFAULTS
            },
        )?;
        click_compare(&app)?;
        app.settle();
        let view = app.window.get_comparison_view();
        if !view.open || view.busy || view.error.as_str() != message {
            return Err(format!(
                "tolerance {text:?}: open={} busy={} error={:?}",
                view.open, view.busy, view.error
            ));
        }
        let main = app.main.clone();
        if !inspect::visible_texts(main.window())
            .iter()
            .any(|shown| shown == message)
        {
            return Err(format!(
                "tolerance {text:?}: the dialog does not show {message:?}"
            ));
        }
        if tools::shown_notice(&app).is_some() {
            return Err(format!(
                "tolerance {text:?} raised a notice: {:?}",
                tools::shown_notice(&app)
            ));
        }
        if text == "abc" {
            app.screenshot(
                context,
                "invalid-tolerance",
                "An invalid tolerance is reported in the dialog",
            );
        }
        input::key(app.slint(), Key::Escape);
        app.until("the Compare dialog to close", |app| {
            !app.window.get_comparison_view().open
        });
    }

    // Nested trees: comparison never recurses, so compare inside the shared folder.
    panel::enter(&mut app, PaneId::Left, "dir-both")?;
    panel::enter(&mut app, PaneId::Right, "dir-both")?;
    let both = compare(&mut app, DEFAULTS)?;
    expect("inside dir-both", &both, &[], &["inner.txt"])?;
    let both = compare(
        &mut app,
        Options {
            directories: true,
            ..DEFAULTS
        },
    )?;
    expect(
        "inside dir-both with directories",
        &both,
        &[],
        &["inner.txt"],
    )?;
    panel::enter(&mut app, PaneId::Left, "deeper")?;
    panel::enter(&mut app, PaneId::Right, "deeper")?;
    let both = compare(&mut app, DEFAULTS)?;
    expect(
        "inside dir-both/deeper",
        &both,
        &["left.txt"],
        &["right.txt"],
    )?;
    app.screenshot(
        context,
        "nested-deeper",
        "Two levels down, each side's only file is selected",
    );
    app.shutdown()?;

    unreadable(context, &root)?;
    running(context, &root)?;
    context.note(
        "identical, asymmetric (size, time, tolerance 0/1/2, case, unmatched folders, contents, subfolders), nested and unreadable trees; invalid tolerances stay in the dialog; Compare closed the dialog at once and no result message appeared",
    );
    Ok(())
}

/// A sparse file of `length` bytes (zeros, no disk space) with the base time.
fn sparse(path: &Path, length: u64) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    tools::sparse(path, length)?;
    tools::set_modified(path, at(0))
}

/// A content comparison long enough to watch: progress, the refusal of Create folder
/// while it runs, the reopened dialog's progress and Cancel.
fn running(context: &mut Context, root: &Path) -> Result<(), String> {
    let (left, right) = (root.join("slow/left"), root.join("slow/right"));
    for side in [&left, &right] {
        sparse(&side.join("big.bin"), 24 << 30)?;
    }
    let mut app = App::start(Config::new(&left, &right));
    open_dialog(&mut app)?;
    set_options(
        &mut app,
        Options {
            contents: true,
            ..DEFAULTS
        },
    )?;
    click_compare(&app)?;
    if app.window.get_comparison_view().open {
        return Err("the dialog stayed open after Compare".into());
    }
    // The status line follows the content comparison.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let progress = loop {
        app.settle();
        let status = app.window.get_desktop_status().to_string();
        if status.starts_with("Comparing contents:") {
            break status;
        }
        if !app.window.get_comparison_view().busy {
            return Err(format!(
                "the 48 GiB comparison ended before its progress showed (status {status:?})"
            ));
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("no progress on the status line: {status:?}"));
        }
        app.idle(Duration::from_millis(50));
    };
    app.screenshot(
        context,
        "running-progress",
        &format!("A running content comparison: {progress}"),
    );
    // Create folder is refused with the reason while it runs, and so is every other
    // action that changes the compared folders or what they show.
    panel::focus_list(&mut app, PaneId::Left)?;
    input::key(app.slint(), Key::F7);
    app.settle();
    let refusal = tools::shown_notice(&app);
    match &refusal {
        Some((title, body))
            if title == "Directory comparison running" && body.contains("cancel") =>
        {
            input::key(app.slint(), Key::Escape);
            app.settle();
        }
        other => {
            return Err(format!(
                "F7 during the comparison: notice {other:?}, expected the refusal"
            ));
        }
    }
    panel::select_row(&mut app, PaneId::Left, "big.bin")?;
    let mut refused = Vec::new();
    for (modifiers, key, how) in [
        (&[][..], slint::SharedString::from(Key::F2), "F2"),
        (&[Key::Shift][..], Key::F4.into(), "Shift+F4"),
        (&[][..], Key::Delete.into(), "Delete"),
        (&[][..], Key::F5.into(), "F5"),
        (&[Key::Control][..], "v".into(), "Ctrl+V"),
        (&[Key::Control][..], "h".into(), "Ctrl+H"),
    ] {
        input::chord(app.slint(), modifiers, key);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let notice = loop {
            app.settle();
            if let Some(notice) = tools::shown_notice(&app) {
                break Some(notice);
            }
            // A dialog instead of the refusal: report it, then close it.
            if !app.window.get_copy_draft_token().is_empty()
                || !app.window.get_operation_token().is_empty()
                || std::time::Instant::now() >= deadline
            {
                break None;
            }
            app.idle(Duration::from_millis(20));
        };
        match notice {
            Some((title, _)) if title == "Directory comparison running" => refused.push(how),
            other => {
                let dialog = !app.window.get_copy_draft_token().is_empty()
                    || !app.window.get_operation_token().is_empty();
                input::key(app.slint(), Key::Escape);
                app.settle();
                return Err(format!(
                    "{how} during the comparison: notice {other:?}, a dialog open {dialog}; expected the refusal \"Directory comparison running\""
                ));
            }
        }
        input::key(app.slint(), Key::Escape);
        app.until("the refusal to close", |app| {
            tools::shown_notice(app).is_none()
        });
        panel::focus_list(&mut app, PaneId::Left)?;
    }
    // The reopened dialog shows the progress and Cancel; Cancel ends the comparison.
    panel::focus_list(&mut app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], Key::F10);
    app.until("the dialog with the running comparison", |app| {
        app.window.get_comparison_view().open
    });
    let main = app.main.clone();
    let texts = inspect::visible_texts(main.window());
    if !texts
        .iter()
        .any(|text| text.starts_with("Comparing contents:"))
    {
        return Err(format!("the reopened dialog shows no progress: {texts:?}"));
    }
    app.screenshot(
        context,
        "running-dialog",
        "Compare directories reopened while it runs: progress and Cancel",
    );
    input::click_button(main.window(), "Cancel")?;
    app.until("Cancel to end the comparison", |app| {
        !app.window.get_comparison_view().busy && !app.window.get_comparison_view().open
    });
    let both = [selected(&app, PaneId::Left), selected(&app, PaneId::Right)];
    if !both[0].is_empty() || !both[1].is_empty() {
        return Err(format!("a cancelled comparison selected {both:?}"));
    }
    // The next comparison runs normally: equal size and time, nothing differs.
    let both = compare(&mut app, DEFAULTS)?;
    expect("after a cancelled comparison", &both, &[], &[])?;
    // The toolbar's Compare directories opens the dialog too; Escape in the dialog of a
    // running comparison cancels it, and nothing is selected.
    panel::focus_list(&mut app, PaneId::Left)?;
    let main = app.main.clone();
    input::click_button(main.window(), "Compare directories")?;
    app.until(
        "the toolbar's Compare directories to open the dialog",
        |app| app.window.get_comparison_view().open,
    );
    set_options(
        &mut app,
        Options {
            contents: true,
            ..DEFAULTS
        },
    )?;
    click_compare(&app)?;
    app.until("the second long comparison to run", |app| {
        app.window
            .get_desktop_status()
            .starts_with("Comparing contents:")
    });
    panel::focus_list(&mut app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], Key::F10);
    app.until("the dialog of the running comparison", |app| {
        app.window.get_comparison_view().open && app.window.get_comparison_view().busy
    });
    input::key(main.window(), Key::Escape);
    app.until("Escape to cancel the comparison", |app| {
        !app.window.get_comparison_view().busy && !app.window.get_comparison_view().open
    });
    let both = [selected(&app, PaneId::Left), selected(&app, PaneId::Right)];
    if !both[0].is_empty() || !both[1].is_empty() {
        return Err(format!(
            "a comparison cancelled with Escape selected {both:?}"
        ));
    }
    context.note(format!(
        "Long comparison: {progress:?}; refused F7: {refusal:?}, and {refused:?}; the toolbar button opened the dialog; Escape cancelled a running comparison, nothing selected"
    ));
    app.shutdown()
}

/// A folder that can be listed but not searched (mode 0444): its entries have no
/// readable metadata, so every pair counts as unknown and is selected on both sides.
#[cfg(unix)]
fn unreadable(context: &mut Context, root: &Path) -> Result<(), String> {
    let left = root.join("unreadable/left");
    let right = root.join("unreadable/right");
    for side in [&left, &right] {
        for name in ["x.txt", "y.txt"] {
            tools::write_at(&side.join(name), b"same", at(0))?;
        }
    }
    tools::set_mode(&left, 0o444)?;
    let mut app = App::start(Config::new(&left, &right));
    let shown = app.names(PaneId::Left);
    let error = app.error(PaneId::Left);
    if !error.is_empty() {
        context.note(format!(
            "the unreadable folder shows the panel error {error:?}"
        ));
    }
    if !(shown.contains(&"x.txt".to_owned()) && shown.contains(&"y.txt".to_owned())) {
        tools::set_mode(&left, 0o755)?;
        return Err(format!(
            "the panel lists {shown:?} for a folder that can be listed but not searched (error {error:?})"
        ));
    }
    let both = compare(&mut app, DEFAULTS);
    tools::set_mode(&left, 0o755)?;
    let both = both?;
    app.screenshot(
        context,
        "unreadable",
        "Entries without readable metadata are selected on both sides",
    );
    expect(
        "unreadable metadata",
        &both,
        &["x.txt", "y.txt"],
        &["x.txt", "y.txt"],
    )?;
    let text = summary(&mut app, context, None)?;
    if !text.is_empty() && !text.contains("Unknown metadata: 2.") {
        return Err(format!("the unreadable comparison summary reads {text:?}"));
    }
    app.shutdown()?;

    // A dangling link has no file metadata either: it and its same-named file are
    // both selected as unknown; an ordinary equal pair is not.
    let left = root.join("special/left");
    let right = root.join("special/right");
    for side in [&left, &right] {
        tools::write_at(&side.join("same.txt"), b"same", at(0))?;
    }
    std::os::unix::fs::symlink("missing-target", left.join("link.txt"))
        .map_err(|error| error.to_string())?;
    tools::write_at(&right.join("link.txt"), b"file", at(0))?;
    let mut app = App::start(Config::new(&left, &right));
    let both = compare(&mut app, DEFAULTS)?;
    expect("a dangling link", &both, &["link.txt"], &["link.txt"])?;
    let text = summary(&mut app, context, None)?;
    if !text.is_empty() && !text.contains("Unknown metadata: 1.") {
        return Err(format!("the dangling link summary reads {text:?}"));
    }
    app.shutdown()
}

#[cfg(not(unix))]
fn unreadable(context: &mut Context, _: &Path) -> Result<(), String> {
    context.note(
        "not on Windows: unreadable metadata. A folder that lists but whose entries cannot \
         be reached (mode 0444) has no Windows equivalent (traverse checking is bypassed), \
         and a dangling file link needs a privilege or Developer Mode",
    );
    Ok(())
}
