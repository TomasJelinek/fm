//! Ctrl+F2 Attributes and times: permission check boxes and the octal field (Windows:
//! the Hidden, Archive and Read-only attributes), and the
//! typed date, time and UTC-offset fields of Modified and Accessed for one file, a
//! multi-selection and a folder. Every change is reviewed, applied and verified on
//! disk; no summary window stays open afterwards.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, io_error},
    input, inspect, panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{MutationKind, OperationProgress, OperationState, PaneId};
use slint::platform::Key;
use std::{
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Seconds since the epoch of a civil UTC date and time (Howard Hinnant's algorithm).
fn civil_seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    days * 86_400 + hour * 3_600 + minute * 60 + second
}

/// The instant a typed date, time and offset (minutes east of UTC) denote.
fn instant(date: (i64, i64, i64), time: (i64, i64, i64), nanos: u32, offset: i64) -> SystemTime {
    let seconds = civil_seconds(date.0, date.1, date.2, time.0, time.1, time.2) - offset * 60;
    UNIX_EPOCH + Duration::new(seconds as u64, nanos)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| io_error(path, error))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<(), String> {
    Ok(())
}

fn open_editor(app: &mut App) -> Result<(), String> {
    panel::wait_input(app);
    input::chord(app.slint(), &[Key::Control], Key::F2);
    // The editor is usable once the captured items' capabilities and current modes
    // have arrived from the backend.
    app.until("the Attributes dialog to load the selection", |app| {
        !app.window.get_metadata_token().is_empty()
            && !app.window.get_metadata_busy()
            && app.window.get_metadata_can_modified()
            && (!cfg!(unix) || app.window.get_metadata_unix_known_mask() != 0)
    });
    Ok(())
}

/// Review changes, Apply, wait for the metadata job; the editor closes and nothing
/// else (no summary or notice) remains open.
fn review_and_apply(
    app: &mut App,
    context: &mut Context,
    shot: Option<&str>,
) -> Result<Arc<OperationProgress>, String> {
    let start = app.operations.len();
    input::click_button(app.slint(), "Review changes")?;
    app.until("the reviewed draft", |app| {
        app.window.get_metadata_preview_current()
            && !app.window.get_metadata_busy()
            && inspect::find(app.slint(), AccessibleRole::Button, "Apply").is_some()
    });
    let error = app.window.get_metadata_error().to_string();
    if !error.is_empty() {
        return Err(format!("the review reported {error:?}"));
    }
    if let Some(name) = shot {
        app.screenshot(context, name, "Attributes › Review changes");
    }
    input::click_button(app.slint(), "Apply")?;
    app.until("the metadata job to finish", move |app| {
        app.operations[start..].iter().any(|progress| {
            progress.kind == MutationKind::EditMetadata && progress.state.is_terminal()
        })
    });
    let progress = app.operations[start..]
        .iter()
        .find(|progress| {
            progress.kind == MutationKind::EditMetadata && progress.state.is_terminal()
        })
        .cloned()
        .ok_or("no metadata report")?;
    app.until(
        "the editor to close and input to return",
        support::main_idle,
    );
    app.idle(Duration::from_millis(400));
    if let Some(notice) = support::shown_notice(app) {
        return Err(format!("a notice stayed open after Apply: {notice:?}"));
    }
    if !app.tool_windows().is_empty() {
        return Err(format!(
            "{} window(s) stayed open after Apply",
            app.tool_windows().len()
        ));
    }
    if progress.state != OperationState::Completed {
        return Err(format!(
            "the metadata job ended {:?}: {:?}",
            progress.state, progress.errors
        ));
    }
    Ok(progress)
}

/// Type a timestamp into the Times page's row `label` (Modified, Accessed).
fn type_time(
    app: &mut App,
    label: &str,
    date: &str,
    time: &str,
    offset: &str,
) -> Result<(), String> {
    input::fill(app.slint(), &format!("{label} date"), date)?;
    input::fill(app.slint(), &format!("{label} time"), time)?;
    input::fill(app.slint(), &format!("{label} UTC offset"), offset)?;
    app.settle();
    Ok(())
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    support::facts(path).map_or(0, |facts| facts.mode)
}

pub fn attributes_and_times(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "attributes")?;
    for name in ["one.txt", "two.txt", "three.txt"] {
        fs::write(folder.join(name), name).map_err(|error| io_error(&folder, error))?;
        set_mode(&folder.join(name), 0o644)?;
    }
    fs::create_dir(folder.join("dir")).map_err(|error| io_error(&folder, error))?;
    fs::write(folder.join("dir/child.txt"), "child").map_err(|error| io_error(&folder, error))?;
    let child_before = support::facts(&folder.join("dir/child.txt"))?;
    let mut app = App::start(Config::new(&folder, &context.work));
    let pane = PaneId::Left;
    let mut failures = Vec::new();

    #[cfg(unix)]
    permissions(&mut app, context, &folder, &mut failures)?;
    #[cfg(windows)]
    windows_attributes(&mut app, context, &folder, &mut failures)?;

    // Modified and accessed times of one file, to the nanosecond, with offsets.
    panel::select_row(&mut app, pane, "one.txt")?;
    open_editor(&mut app)?;
    input::click_button(app.slint(), "Times")?;
    app.settle();
    type_time(
        &mut app,
        "Modified",
        "2021-03-04",
        "05:06:07.123456789",
        "+02:00",
    )?;
    type_time(&mut app, "Accessed", "2020-01-02", "03:04:05", "Z")?;
    app.screenshot(
        context,
        "times-typed",
        "Times page with typed date, time and offset",
    );
    // NTFS keeps times in 100 ns steps: the nanoseconds typed arrive truncated there.
    let nanos = if cfg!(windows) {
        123_456_700
    } else {
        123_456_789
    };
    let modified = instant((2021, 3, 4), (5, 6, 7), nanos, 120);
    let accessed = instant((2020, 1, 2), (3, 4, 5), 0, 0);
    match review_and_apply(&mut app, context, Some("times-review")) {
        Err(error) => failures.push(format!("times of one.txt: {error}")),
        Ok(_) => {
            let found = support::facts(&folder.join("one.txt"))?;
            if found.modified != modified || found.accessed != accessed {
                failures.push(format!(
                    "one.txt times: modified {:?} (expected {modified:?}), accessed {:?} (expected {accessed:?})",
                    found.modified, found.accessed
                ));
            }
        }
    }
    support::dismiss_all(&mut app);

    // Several files at once.
    for name in ["one.txt", "two.txt", "three.txt"] {
        panel::select_row(&mut app, pane, name)?;
        input::key(app.slint(), Key::Insert);
    }
    app.settle();
    // Only Modified is ticked: the unticked Accessed row must leave each time alone.
    // Metadata only: reading a file's content could move its access time itself.
    let accessed_of = |name: &str| {
        let path = folder.join(name);
        std::fs::metadata(&path)
            .and_then(|metadata| metadata.accessed())
            .map_err(|error| support::io_error(&path, error))
    };
    let mut accessed_before = Vec::new();
    for name in ["one.txt", "two.txt", "three.txt"] {
        accessed_before.push(accessed_of(name)?);
    }
    open_editor(&mut app)?;
    input::click_button(app.slint(), "Times")?;
    app.settle();
    type_time(&mut app, "Modified", "2019-12-31", "23:59:59", "-05:00")?;
    let modified = instant((2019, 12, 31), (23, 59, 59), 0, -300);
    match review_and_apply(&mut app, context, None) {
        Err(error) => failures.push(format!("times of three files: {error}")),
        Ok(_) => {
            for (name, accessed) in ["one.txt", "two.txt", "three.txt"]
                .into_iter()
                .zip(&accessed_before)
            {
                let now = accessed_of(name)?;
                if now != *accessed {
                    failures.push(format!(
                        "{name} accessed {now:?} after Apply with Accessed unticked, was {accessed:?}"
                    ));
                }
                let found = support::facts(&folder.join(name))?;
                if found.modified != modified {
                    failures.push(format!(
                        "{name} modified {:?}, expected {modified:?}",
                        found.modified
                    ));
                }
            }
        }
    }
    support::dismiss_all(&mut app);
    panel::focus_list(&mut app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // A folder: its own time changes, its contents do not.
    panel::select_row(&mut app, pane, "dir")?;
    open_editor(&mut app)?;
    input::click_button(app.slint(), "Times")?;
    app.settle();
    type_time(&mut app, "Modified", "2018-06-07", "08:09:10", "Z")?;
    let modified = instant((2018, 6, 7), (8, 9, 10), 0, 0);
    match review_and_apply(&mut app, context, None) {
        Err(error) => failures.push(format!("time of dir: {error}")),
        Ok(_) => {
            let found = support::facts(&folder.join("dir"))?;
            if found.modified != modified {
                failures.push(format!(
                    "dir modified {:?}, expected {modified:?}",
                    found.modified
                ));
            }
            if support::facts(&folder.join("dir/child.txt"))? != child_before {
                failures.push("changing the folder's time changed dir/child.txt".into());
            }
        }
    }
    app.screenshot(
        context,
        "after-attributes",
        "After the attribute and time changes",
    );
    context.note(format!(
        "Ctrl+F2: {}, modified+accessed of one file with offsets and nanoseconds, modified of three files and of a folder — verified on disk, no window left open",
        if cfg!(windows) {
            "Hidden set and Archive cleared on one.txt, Read-only set on two.txt"
        } else {
            "check boxes (660), octal 444"
        }
    ));
    app.shutdown()?;
    support::report(&failures)
}

/// Permission check boxes (group write set, other read cleared) and the octal field
/// (444), each reviewed, applied and read back from disk.
#[cfg(unix)]
fn permissions(
    app: &mut App,
    context: &mut Context,
    folder: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let pane = PaneId::Left;
    // Permission check boxes: set group write, clear other read.
    panel::select_row(app, pane, "one.txt")?;
    open_editor(app)?;
    app.screenshot(context, "attributes", "Ctrl+F2 Attributes of one.txt");
    input::click(app.slint(), AccessibleRole::Checkbox, "Group write")?;
    input::click(app.slint(), AccessibleRole::Checkbox, "Other read")?;
    app.settle();
    match review_and_apply(app, context, Some("attributes-review")) {
        Err(error) => failures.push(format!("permissions of one.txt: {error}")),
        Ok(_) if mode(&folder.join("one.txt")) != 0o660 => failures.push(format!(
            "one.txt has mode {:o} instead of 660",
            mode(&folder.join("one.txt"))
        )),
        Ok(_) => {}
    }
    support::dismiss_all(app);

    // The octal field: read-only for everyone.
    panel::select_row(app, pane, "two.txt")?;
    open_editor(app)?;
    input::fill(app.slint(), "Octal permissions", "444")?;
    app.settle();
    match review_and_apply(app, context, None) {
        Err(error) => failures.push(format!("octal 444 on two.txt: {error}")),
        Ok(_) if mode(&folder.join("two.txt")) != 0o444 => failures.push(format!(
            "two.txt has mode {:o} instead of 444",
            mode(&folder.join("two.txt"))
        )),
        Ok(_) => {}
    }
    support::dismiss_all(app);
    Ok(())
}

/// Windows attributes: Hidden set and Archive cleared on one.txt, Read-only set on
/// two.txt, each reviewed, applied and read back from disk (the file's attribute bits).
#[cfg(windows)]
fn windows_attributes(
    app: &mut App,
    context: &mut Context,
    folder: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    const READ_ONLY: u32 = 0x1;
    const HIDDEN: u32 = 0x2;
    const ARCHIVE: u32 = 0x20;
    let attributes = |name: &str| {
        fs::metadata(folder.join(name)).map_or(0, |metadata| metadata.file_attributes())
    };
    let pane = PaneId::Left;
    for (name, changes, wanted, unwanted) in [
        (
            "one.txt",
            &[("Hidden", "Set"), ("Archive", "Clear")][..],
            HIDDEN,
            ARCHIVE | READ_ONLY,
        ),
        ("two.txt", &[("Read-only", "Set")][..], READ_ONLY, HIDDEN),
    ] {
        panel::select_row(app, pane, name)?;
        open_editor(app)?;
        if name == "one.txt" {
            app.screenshot(context, "attributes", "Ctrl+F2 Attributes of one.txt");
        }
        // Each attribute is a three-state box: a click goes Unchanged, Set, Clear.
        for (label, choice) in changes {
            let state = |app: &App| {
                inspect::find(app.slint(), AccessibleRole::Checkbox, label)
                    .map(|item| crate::tools_support::value(&item))
            };
            for _ in 0..3 {
                if state(app).as_deref() == Some(*choice) {
                    break;
                }
                input::click(app.slint(), AccessibleRole::Checkbox, label)?;
                app.settle();
            }
            if state(app).as_deref() != Some(*choice) {
                return Err(format!(
                    "the {label:?} attribute shows {:?}, not {choice:?}",
                    state(app)
                ));
            }
        }
        app.settle();
        let shot = (name == "one.txt").then_some("attributes-review");
        match review_and_apply(app, context, shot) {
            Err(error) => failures.push(format!("attributes of {name}: {error}")),
            Ok(_) => {
                let found = attributes(name);
                if found & wanted != wanted || found & unwanted != 0 {
                    failures.push(format!(
                        "{name} has the attributes {found:#x} after {changes:?}"
                    ));
                }
            }
        }
        support::dismiss_all(app);
    }
    Ok(())
}
