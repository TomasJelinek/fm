//! Compare directories (Ctrl+F10), by Open Salamander's rules: every option of the
//! dialog on identical, asymmetric, nested and unreadable trees, contents and
//! subfolders included. A metadata comparison closes the dialog at once; one with
//! contents or subfolders keeps the dialog open with "Comparing … with …", its counts,
//! a progress bar and Cancel, then closes it. The differences become each panel's
//! selection (a differing folder in both panels), one line below the active panel
//! reports the result, and no line appears at the bottom of the window. With
//! subfolders every compared row shows whether it matches, also inside an expanded
//! folder. Invalid options are reported in the dialog, which stays open. A long
//! content comparison (two 8 GiB sparse files, deleted afterwards) is cancelled by
//! the Cancel button, by Escape and by Alt+C within a moment, selecting nothing; the
//! next comparison runs normally.
use crate::{
    Context,
    app::{App, Config},
    files_support, input, inspect,
    options_support::SettingsStore,
    panel, tools_support as tools,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, PreferenceTheme};
use slint::platform::{Key, WindowAdapter};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
    tools::write_at(&root.join("left/dir-both/equal/e.txt"), b"e", at(5))?;
    tools::write_at(&root.join("right/dir-both/equal/e.txt"), b"e", at(5))?;
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
    // Compare subfolders disables Select one-sided folders, and an unticked time the
    // tolerance field: clear the one, tick the other first.
    tools::set_check(app, window, "Compare subfolders", false)?;
    tools::set_check(app, window, "Compare modification time", true)?;
    input::fill(window, "Time tolerance in seconds", options.tolerance)?;
    for (label, wanted) in [
        ("Compare file size", options.size),
        ("Compare modification time", options.time),
        ("Compare contents", options.contents),
        ("Match names with exact case", options.exact_case),
        ("Select one-sided folders", options.directories),
        ("Compare subfolders", options.subfolders),
    ] {
        tools::set_check(app, window, label, wanted)?;
    }
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

/// The result line below the left panel, where the comparisons start.
fn result_line(app: &App) -> String {
    app.window.get_left_watch_status().to_string()
}

/// No line at the bottom of the window reports a comparison (the owner's report).
fn no_bottom_line(app: &App) -> Result<(), String> {
    let status = app.window.get_desktop_status().to_string();
    if status.starts_with("Compar")
        || app.window.get_desktop_feedback_important() && status.contains("ompar")
    {
        return Err(format!(
            "the bottom line reports the comparison: {status:?}"
        ));
    }
    Ok(())
}

/// Compare with `options` and return both panels' selections. A metadata comparison
/// closes the dialog at the click; one with contents or subfolders keeps it open with
/// its progress until it ends. No notice or other window reports the result: one line
/// below the left panel does.
fn compare(app: &mut App, options: Options) -> Result<[BTreeSet<String>; 2], String> {
    open_dialog(app)?;
    set_options(app, options)?;
    let tools_before = app.tool_windows().len();
    click_compare(app)?;
    let long = options.contents || options.subfolders;
    // The callback publishes the dialog's state synchronously, while the Engine works.
    let view = app.window.get_comparison_view();
    if !view.error.is_empty() {
        return Err(format!("Compare was refused: {}", view.error));
    }
    if view.open != long || view.running != long {
        return Err(format!(
            "after Compare the dialog is open={} running={} (contents or subfolders: {long})",
            view.open, view.running
        ));
    }
    app.until("the comparison result", |app| {
        let view = app.window.get_comparison_view();
        !view.busy && (!view.open || !view.error.is_empty() || !view.summary.is_empty())
    });
    let view = app.window.get_comparison_view();
    if !view.error.is_empty() {
        return Err(format!("the comparison failed: {}", view.error));
    }
    if view.open {
        return Err(format!("the dialog stayed open: {}", view.summary));
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
    no_bottom_line(app)?;
    let both = [selected(app, PaneId::Left), selected(app, PaneId::Right)];
    let line = result_line(app);
    let wanted = if both[0].is_empty() && both[1].is_empty() {
        "Compared: The files in the two folders".to_owned()
    } else {
        format!(
            "Compared: {} selected here, {} in the right panel",
            both[0].len(),
            both[1].len()
        )
    };
    if !line.starts_with(&wanted) {
        return Err(format!(
            "below the left panel: {line:?}; the panels show {} and {} selected",
            both[0].len(),
            both[1].len()
        ));
    }
    if !inspect::visible_texts(app.slint()).contains(&line) {
        return Err(format!("the result line {line:?} is not shown"));
    }
    Ok(both)
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

fn line_has(app: &App, case: &str, parts: &[&str]) -> Result<(), String> {
    let line = result_line(app);
    let details = app.window.get_left_watch_details().to_string();
    for part in parts {
        if !line.contains(part) && !details.contains(part) {
            return Err(format!(
                "{case}: {part:?} is neither in {line:?} nor in its details {details:?}"
            ));
        }
    }
    Ok(())
}

/// Each panel row's comparison mark: 1 matches, 2 differs, 3 this panel only.
fn marks(app: &App, pane: PaneId) -> Vec<(String, i32)> {
    app.rows(pane)
        .iter()
        .filter(|row| !row.parent_entry)
        .map(|row| (row.name.to_string(), row.compare_mark))
        .collect()
}

fn expect_marks(app: &App, case: &str, pane: PaneId, wanted: &[(&str, i32)]) -> Result<(), String> {
    let shown = marks(app, pane);
    for (name, mark) in wanted {
        let found = shown
            .iter()
            .find(|(row, _)| row == name)
            .map(|(_, mark)| *mark);
        if found != Some(*mark) {
            return Err(format!(
                "{case}: {name} has mark {found:?}, expected {mark}; rows {shown:?}"
            ));
        }
    }
    Ok(())
}

pub fn compare_directories(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("compare");
    let outcome = compare_all(context, &root);
    // Leave nothing large behind in the work folder.
    let _ = fs::remove_dir_all(root.join("slow"));
    outcome
}

fn compare_all(context: &mut Context, root: &Path) -> Result<(), String> {
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
    line_has(&app, "identical trees", &["appear to be identical"])?;
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
    // Contents reads the two same-sized pairs with bytes (an empty pair is equal
    // unread) and subfolders walk the matched folder: identical, no difference.
    let both = compare(
        &mut app,
        Options {
            contents: true,
            subfolders: true,
            ..DEFAULTS
        },
    )?;
    expect("identical trees, contents and subfolders", &both, &[], &[])?;
    line_has(
        &app,
        "identical contents",
        &["are identical", "Files read byte by byte: 2 pairs"],
    )?;
    expect_marks(
        &app,
        "identical marks",
        PaneId::Right,
        &[("a.txt", 1), ("c.md", 1), ("folder", 1)],
    )?;
    app.shutdown()?;

    let left = root.join("asymmetric/left");
    let right = root.join("asymmetric/right");
    let mut app = App::start(Config::new(&left, &right));
    open_dialog(&mut app)?;
    app.screenshot(
        context,
        "dialog-12-light",
        "Compare directories, 12 px, light",
    );
    input::key(app.slint(), Key::Escape);
    app.until("the dialog to close", |app| {
        !app.window.get_comparison_view().open
    });
    // Size and time with the default two-second tolerance and exact case.
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
    let both = compare(&mut app, DEFAULTS)?;
    app.screenshot(
        context,
        "asymmetric-default",
        "Differences selected in both panels (size, time, exact case); the result below the active panel",
    );
    expect("defaults", &both, &default_left, &default_right)?;
    let status = app.window.get_left_status().to_string();
    if !status.contains("4 selected") {
        return Err(format!("the left status reads {status:?}"));
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
        &default_right,
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
        &default_right,
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
    // One-sided folders join the selection; the matched dir-both is not compared.
    let both = compare(
        &mut app,
        Options {
            directories: true,
            ..DEFAULTS
        },
    )?;
    expect(
        "one-sided folders",
        &both,
        &[&default_left[..], &["dir-left-only"]].concat(),
        &[&default_right[..], &["dir-right-only"]].concat(),
    )?;

    // Content (Salamander): different sizes select both files; equal sizes are read
    // unless both files are selected already ('l' against 'r' differ). The newer
    // pairs are read too, so both of their files end up selected.
    let both_sides = [
        "bigger-left.dat",
        "bigger-right.dat",
        "newer-left.txt",
        "newer-right.txt",
        "same.txt",
        "within-tolerance.txt",
        "at-tolerance.txt",
    ];
    let contents_left = [&["only-left.txt", "README.TXT"][..], &both_sides].concat();
    let contents_right = [&["only-right.txt", "readme.txt"][..], &both_sides].concat();
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
        "Content: different sizes and different bytes select both files",
    );
    expect("contents", &both, &contents_left, &contents_right)?;
    line_has(
        &app,
        "contents",
        &[
            "5 with different contents",
            "Files read byte by byte: 5 pairs",
        ],
    )?;
    // Subfolders: dir-both differs inside, so it is selected in both panels;
    // one-sided folders always count. Each row shows how it compared.
    let both = compare(
        &mut app,
        Options {
            subfolders: true,
            ..DEFAULTS
        },
    )?;
    expect(
        "subfolders",
        &both,
        &[&default_left[..], &["dir-both", "dir-left-only"]].concat(),
        &[&default_right[..], &["dir-both", "dir-right-only"]].concat(),
    )?;
    line_has(&app, "subfolders", &["1 of 1 folders differ"])?;
    expect_marks(
        &app,
        "subfolder marks, left",
        PaneId::Left,
        &[
            ("dir-both", 2),
            ("dir-left-only", 3),
            ("same.txt", 1),
            ("within-tolerance.txt", 1),
            ("bigger-left.dat", 2),
            ("README.TXT", 3),
        ],
    )?;
    expect_marks(
        &app,
        "subfolder marks, right",
        PaneId::Right,
        &[
            ("dir-both", 2),
            ("dir-right-only", 3),
            ("newer-right.txt", 2),
        ],
    )?;
    let description = app
        .rows(PaneId::Left)
        .iter()
        .any(|row| row.name == "dir-both" && row.compare_mark == 2);
    if !description {
        return Err("dir-both shows no mark".into());
    }
    app.screenshot(
        context,
        "asymmetric-subfolders",
        "Compare subfolders: the differing folder is selected in both panels; every row shows = ≠ +",
    );
    // Expanded, dir-both shows which of its subfolders differ.
    files_support::toggle_tree(&mut app, "dir-both")?;
    files_support::wait_tree_depth(&mut app, "deeper", Some(1));
    expect_marks(
        &app,
        "inside the expanded dir-both",
        PaneId::Left,
        &[("dir-both", 2), ("deeper", 2), ("equal", 1)],
    )?;
    app.screenshot(
        context,
        "asymmetric-subfolders-expanded",
        "Inside the expanded folder, deeper differs and equal matches",
    );
    files_support::toggle_tree(&mut app, "dir-both")?;
    files_support::wait_tree_depth(&mut app, "deeper", None);
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
        &[&contents_left[..], &["dir-both", "dir-left-only"]].concat(),
        &[&contents_right[..], &["dir-both", "dir-right-only"]].concat(),
    )?;

    // Invalid tolerances are reported in the dialog, which stays open; nothing runs.
    let message = "Enter a whole number of seconds from 0 to 86400 for the time tolerance.";
    for text in ["abc", "86401"] {
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

    // Without subfolders nothing recurses: compare inside the shared folder.
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
        "inside dir-both with one-sided folders",
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
    app.shutdown()?;

    appearance(context, root)?;
    unreadable(context, root)?;
    running(context, root)?;
    context.note(
        "identical, asymmetric (size, time, tolerance 0/1/2, case, one-sided folders, contents, subfolders with marks, expanded marks), nested and unreadable trees; invalid tolerances stay in the dialog; no bottom status line; results below the active panel",
    );
    Ok(())
}

/// The dialog at 8 and 32 px and in the dark theme, and the dark panels' marks.
fn appearance(context: &mut Context, root: &Path) -> Result<(), String> {
    let left = root.join("asymmetric/left");
    let right = root.join("asymmetric/right");
    for (font, theme, name) in [
        (8, PreferenceTheme::Light, "dialog-8-light"),
        (32, PreferenceTheme::Light, "dialog-32-light"),
        (12, PreferenceTheme::Dark, "dialog-12-dark"),
    ] {
        let mut config = Config::new(&left, &right);
        config.settings = Some(SettingsStore::seeded(move |preferences| {
            preferences.desktop_font_size = font;
        }));
        config.theme = theme;
        let mut app = App::start(config);
        open_dialog(&mut app)?;
        let main = app.main.clone();
        for label in ["Compare", "Close"] {
            let button = tools::control(main.window(), AccessibleRole::Button, label)?;
            if !tools::enabled(&button) {
                return Err(format!("{name}: {label} is not available"));
            }
        }
        app.screenshot(
            context,
            name,
            &format!("Compare directories at {font} px, {theme:?}"),
        );
        input::key(app.slint(), Key::Escape);
        app.until("the dialog to close", |app| {
            !app.window.get_comparison_view().open
        });
        if theme == PreferenceTheme::Dark {
            compare(
                &mut app,
                Options {
                    subfolders: true,
                    ..DEFAULTS
                },
            )?;
            files_support::toggle_tree(&mut app, "dir-both")?;
            files_support::wait_tree_depth(&mut app, "deeper", Some(1));
            app.screenshot(
                context,
                "subfolders-dark",
                "Subfolder marks in the dark theme, dir-both expanded",
            );
        }
        app.shutdown()?;
    }
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

/// Starts a long content comparison and waits for its progress in the dialog.
fn start_long(app: &mut App) -> Result<String, String> {
    set_options(
        app,
        Options {
            contents: true,
            ..DEFAULTS
        },
    )?;
    click_compare(app)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.settle();
        let view = app.window.get_comparison_view();
        if view.running
            && view
                .progress_counts
                .starts_with("Comparing the displayed files")
        {
            if !view.progress_detail.is_empty() || Instant::now() >= deadline {
                return Ok(format!(
                    "{} · {}",
                    view.progress_counts, view.progress_detail
                ));
            }
        } else if !view.busy {
            return Err(format!(
                "the long comparison ended before its progress showed: {:?}",
                view.error
            ));
        }
        if Instant::now() >= deadline {
            return Err("no progress in the dialog".into());
        }
        app.idle(Duration::from_millis(50));
    }
}

/// How long the comparison took to end after `cancel`, which must select nothing.
fn cancelled_by(
    app: &mut App,
    how: &str,
    cancel: impl FnOnce(&mut App),
) -> Result<Duration, String> {
    let started = Instant::now();
    cancel(app);
    app.until(&format!("{how} to end the comparison"), |app| {
        let view = app.window.get_comparison_view();
        !view.busy && !view.open
    });
    let took = started.elapsed();
    let both = [selected(app, PaneId::Left), selected(app, PaneId::Right)];
    if !both[0].is_empty() || !both[1].is_empty() {
        return Err(format!("a comparison cancelled by {how} selected {both:?}"));
    }
    no_bottom_line(app)?;
    Ok(took)
}

/// A content comparison long enough to watch: the dialog's progress, and Cancel,
/// Escape and Alt+C each ending it at once without selecting anything.
fn running(context: &mut Context, root: &Path) -> Result<(), String> {
    let (left, right) = (root.join("slow/left"), root.join("slow/right"));
    for side in [&left, &right] {
        sparse(&side.join("big.bin"), 8 << 30)?;
    }
    let mut app = App::start(Config::new(&left, &right));
    open_dialog(&mut app)?;
    let progress = start_long(&mut app)?;
    let main = app.main.clone();
    let texts = inspect::visible_texts(main.window());
    for wanted in ["Comparing", "with", "Comparing the displayed files"] {
        if !texts.iter().any(|text| text.starts_with(wanted)) {
            return Err(format!("the progress shows no {wanted:?}: {texts:?}"));
        }
    }
    no_bottom_line(&app)?;
    app.screenshot(
        context,
        "running-progress",
        &format!("A running content comparison: {progress}"),
    );
    let button = cancelled_by(&mut app, "the Cancel button", |app| {
        let main = app.main.clone();
        let _ = input::click_button(main.window(), "Cancel");
    })?;
    // The next comparison runs normally: equal size and time, nothing differs.
    let both = compare(&mut app, DEFAULTS)?;
    expect("after a cancelled comparison", &both, &[], &[])?;
    // The toolbar's Compare directories opens the dialog too; Escape cancels.
    panel::focus_list(&mut app, PaneId::Left)?;
    let main = app.main.clone();
    input::click_button(main.window(), "Compare directories")?;
    app.until(
        "the toolbar's Compare directories to open the dialog",
        |app| app.window.get_comparison_view().open,
    );
    start_long(&mut app)?;
    let escape = cancelled_by(&mut app, "Escape", |app| {
        input::key(app.slint(), Key::Escape);
    })?;
    open_dialog(&mut app)?;
    start_long(&mut app)?;
    let alt_c = cancelled_by(&mut app, "Alt+C", |app| {
        input::chord(app.slint(), &[Key::Alt], "c");
    })?;
    context.note(format!(
        "Long comparison: {progress:?}; ended after Cancel in {button:?}, Escape in {escape:?}, Alt+C in {alt_c:?}; nothing selected"
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
    line_has(
        &app,
        "unreadable metadata",
        &["Unknown metadata or not verified: 2."],
    )?;
    app.shutdown()?;

    // A dangling link opposite a file of its name differs: both are selected; an
    // ordinary equal pair is not.
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
