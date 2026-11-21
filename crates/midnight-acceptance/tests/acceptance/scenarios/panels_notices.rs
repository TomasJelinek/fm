//! Panel error notices for an unreadable folder at the end of a long path: entered with
//! Enter, with a double click, as a typed path (Ctrl+G), a typed path that does not
//! exist, and as an inline branch that cannot be listed. Each notice names the folder
//! and shows its whole path (the dialog grows instead of cutting lines), Details expands
//! and collapses, Copy details copies the report, every way to close it (Escape, Close,
//! the header's ×) leaves the panel on its last listing with nothing else open, and the
//! same holds at touch density in a smaller window. A background probe of the folder
//! never opens a notice.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use midnight_core::UiDensity;
use midnight_ui::NoticePresentation;
use slint::{ComponentHandle, platform::Key};
use std::{path::Path, time::Duration};

const LOCKED: &str = "locked folder";
const TITLE: &str = "Cannot open folder";
const DENIED: &str = "Access to this folder was denied.";

fn folder_path(app: &App) -> String {
    app.window
        .global::<NoticePresentation>()
        .get_folder_path()
        .to_string()
}

/// The notice that `what` opened: its title, its body, the folder it names, every line
/// readable whole, one close control in its header and one Close button.
#[allow(clippy::too_many_arguments)] // One call per way of opening the notice.
fn check_notice(
    app: &mut App,
    context: &mut Context,
    what: &str,
    path: &Path,
    wanted_title: &str,
    body_part: Option<&str>,
    shot: Option<&str>,
    failures: &mut Vec<String>,
) -> bool {
    if !ps::wait(app, Duration::from_secs(10), |app| {
        support::shown_notice(app).is_some()
    }) {
        failures.push(format!("{what}: no notice within 10 s"));
        return false;
    }
    // The body and folder box are filled in the same update; give layout a frame.
    app.idle(Duration::from_millis(100));
    let (title, body) = support::shown_notice(app).unwrap_or_default();
    if title != wanted_title {
        failures.push(format!(
            "{what}: notice title {title:?}, expected {wanted_title:?}"
        ));
    }
    if let Some(part) = body_part
        && !body.contains(part)
    {
        failures.push(format!("{what}: notice body {body:?} lacks {part:?}"));
    }
    let wanted = path.to_string_lossy().into_owned();
    let named = folder_path(app);
    if named != wanted {
        failures.push(format!(
            "{what}: the notice names the folder {named:?}, expected {wanted:?}"
        ));
    }
    let texts = ps::dialog_texts(app);
    match texts.iter().find(|text| text.text == wanted) {
        None => failures.push(format!(
            "{what}: the notice shows no text with the whole path; texts {:?}",
            texts.iter().map(|text| &text.text).collect::<Vec<_>>()
        )),
        Some(text) if text.cut() => failures.push(format!(
            "{what}: the {}-character path is laid out {:.0} px tall but only {:?} shows: the notice does not grow to show the whole path",
            wanted.chars().count(),
            text.full.3,
            text.visible.map(|visible| visible.3)
        )),
        Some(_) => {}
    }
    let cut: Vec<String> = texts
        .iter()
        .filter(|text| text.cut())
        .map(|text| {
            format!(
                "{:?} ({:.0} px of {:.0} px shown)",
                text.text.chars().take(60).collect::<String>(),
                text.visible.map_or(0.0, |visible| visible.3),
                text.full.3
            )
        })
        .collect();
    if !cut.is_empty() {
        failures.push(format!("{what}: notice text cut off: {cut:?}"));
    }
    let buttons: Vec<String> = ps::popup_buttons(app)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    let headers = buttons
        .iter()
        .filter(|label| ps::HEADER_CLOSE.contains(&label.as_str()))
        .count();
    let closes = buttons.iter().filter(|label| *label == "Close").count();
    if headers != 1 || closes != 1 {
        failures.push(format!(
            "{what}: the notice has {headers} header close control(s) and {closes} Close button(s), expected one each; buttons {buttons:?}"
        ));
    }
    if let Some(name) = shot {
        app.screenshot(context, name, &format!("{what}: {title}"));
    }
    true
}

/// Close the open notice with `how`, then the panel must be back to browsing on
/// `location` with the cursor on `cursor`, without a banner or another notice.
fn close_notice(
    app: &mut App,
    what: &str,
    how: &str,
    location: &Path,
    cursor: Option<&str>,
    failures: &mut Vec<String>,
) {
    let result = match how {
        "Escape" => {
            input::key(app.slint(), Key::Escape);
            Ok(())
        }
        label => ps::click_popup_button(app, label),
    };
    if let Err(error) = result {
        failures.push(format!("{what}: closing with {how}: {error}"));
        support::dismiss_all(app);
        return;
    }
    if !ps::wait(app, Duration::from_secs(5), |app| {
        support::shown_notice(app).is_none()
    }) {
        failures.push(format!("{what}: {how} did not close the notice"));
        support::dismiss_all(app);
        return;
    }
    if let Err(error) = ps::stays_idle(app, Duration::from_millis(600)) {
        failures.push(format!("{what}: after {how}: {error}"));
        support::dismiss_all(app);
    }
    if app.location(LEFT).as_deref() != Some(location) {
        failures.push(format!(
            "{what}: after {how} the panel shows {:?}, expected its last listing {}",
            app.location(LEFT),
            location.display()
        ));
    }
    if let Some(name) = cursor
        && ps::cursor(app, LEFT).as_deref() != Some(name)
    {
        failures.push(format!(
            "{what}: after {how} the cursor is on {:?}, expected {name:?}",
            ps::cursor(app, LEFT)
        ));
    }
    let banner = app.error(LEFT);
    if !banner.is_empty() {
        failures.push(format!(
            "{what}: after {how} the panel also shows an error banner {banner:?} (the notice already reported it)"
        ));
    }
}

fn typed_path(app: &mut App, path: &Path) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &path.to_string_lossy());
    input::key(app.slint(), Key::Return);
    app.settle();
    Ok(())
}

/// Click the expander of `name` whatever it offers (Expand, Discover, Retry).
fn click_expander(app: &mut App, name: &str) -> Result<(), String> {
    let edge = app.window.get_left_list_left() + app.window.get_left_list_width();
    let item = ["Expand", "Discover", "Retry", "Collapse"]
        .iter()
        .find_map(|verb| {
            crate::inspect::find_all(
                app.slint(),
                i_slint_core::items::AccessibleRole::Button,
                &format!("{verb} folders in {name}"),
            )
            .into_iter()
            .find(|item| crate::inspect::center(item).x < edge)
        })
        .ok_or_else(|| format!("no expander for {name:?} in the left panel"))?;
    input::click_at(app.slint(), crate::inspect::center(&item));
    app.settle();
    Ok(())
}

pub fn notices(context: &mut Context) -> Result<(), String> {
    run(context)
}

fn run(context: &mut Context) -> Result<(), String> {
    let base = support::scratch(context, "notices")?;
    let mut parent = base.clone();
    for level in 1..=6 {
        parent = parent.join(format!(
            "a-long-folder-name-for-the-notice-path-check-level-{level:02}"
        ));
    }
    let locked = parent.join(LOCKED);
    ps::folder(&locked.join("inside"))?;
    ps::write(&locked.join("secret.txt"), "secret")?;
    ps::write(&parent.join("readable.txt"), "readable")?;
    let guard = ps::lock(&locked)?;
    let mut failures = Vec::new();
    context.note(format!(
        "the unreadable folder's path has {} characters",
        locked.to_string_lossy().chars().count()
    ));
    desktop(context, &parent, &locked, &mut failures)?;
    touch(context, &parent, &locked, &mut failures)?;
    drop(guard);
    support::report(&failures)
}

fn desktop(
    context: &mut Context,
    parent: &Path,
    locked: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let mut config = Config::new(parent, &context.work);
    config.clipboard = true;
    let mut app = App::start(config);
    // The listing probes the unreadable folder for subfolders: that background
    // failure must not open a notice.
    if let Err(error) = ps::stays_idle(&mut app, Duration::from_millis(800)) {
        failures.push(format!("at startup beside an unreadable folder: {error}"));
        support::dismiss_all(&mut app);
    }

    // Enter on the row.
    panel::select_row(&mut app, LEFT, LOCKED)?;
    input::key(app.slint(), Key::Return);
    if check_notice(
        &mut app,
        context,
        "Enter on the unreadable folder",
        locked,
        TITLE,
        Some(DENIED),
        Some("unreadable-enter"),
        failures,
    ) {
        close_notice(&mut app, "Enter", "Escape", parent, Some(LOCKED), failures);
    }
    let listed: Vec<String> = {
        let mut names = ps::shown(&app, LEFT);
        names.sort();
        names
    };
    let disk = panel::disk_names(parent);
    if listed != disk {
        failures.push(format!(
            "after the failed Enter the panel lists {listed:?}, the folder holds {disk:?}"
        ));
    }

    // A double click: Details expands and collapses, Copy details copies, Close.
    panel::wait_input(&mut app);
    let position = panel::row_center(&app, LEFT, LOCKED).ok_or("no row for the locked folder")?;
    ps::double_click(&mut app, position);
    if check_notice(
        &mut app,
        context,
        "double click on the unreadable folder",
        locked,
        TITLE,
        Some(DENIED),
        None,
        failures,
    ) {
        details(&mut app, context, locked, failures);
        close_notice(
            &mut app,
            "double click",
            "Close",
            parent,
            Some(LOCKED),
            failures,
        );
    }

    // A typed path, closed with the header's ×.
    typed_path(&mut app, locked)?;
    if check_notice(
        &mut app,
        context,
        "Ctrl+G to the unreadable folder",
        locked,
        TITLE,
        Some(DENIED),
        None,
        failures,
    ) {
        close_notice(&mut app, "Ctrl+G", "Close dialog", parent, None, failures);
    }
    // A typed path that does not exist names the missing folder.
    let missing = parent.join("this folder does not exist");
    typed_path(&mut app, &missing)?;
    if check_notice(
        &mut app,
        context,
        "Ctrl+G to a missing folder",
        &missing,
        TITLE,
        None,
        Some("missing-folder"),
        failures,
    ) {
        close_notice(&mut app, "Ctrl+G missing", "Escape", parent, None, failures);
    }
    // Leave the path editor if the failed navigation left it focused.
    if app.window.get_focused_pane() != 0 {
        input::key(app.slint(), Key::Escape);
        app.settle();
    }

    // An inline branch that cannot be listed: its red icon opens the same notice.
    tree_error(&mut app, context, parent, locked, failures)?;
    app.shutdown()
}

/// Details expands the diagnostic report and collapses it again; Copy details puts the
/// whole report, with the reason and the folder's path, on the clipboard and keeps the
/// notice open.
fn details(app: &mut App, context: &mut Context, locked: &Path, failures: &mut Vec<String>) {
    let report_shown = |app: &App| {
        ps::dialog_texts(app)
            .iter()
            .any(|text| text.text.contains("Error kind"))
    };
    if report_shown(app) {
        failures.push("the notice shows its diagnostic report before Details".into());
    }
    if let Err(error) = ps::click_popup_button(app, "Details") {
        failures.push(format!("notice Details: {error}"));
        return;
    }
    if !ps::wait(app, Duration::from_secs(3), report_shown) {
        failures.push("Details did not show the diagnostic report (Operation, Error kind)".into());
    } else {
        app.screenshot(
            context,
            "unreadable-details",
            "The notice with Details expanded",
        );
        if let Err(error) = ps::click_popup_button(app, "Hide details") {
            failures.push(format!("notice Hide details: {error}"));
        } else if !ps::wait(app, Duration::from_secs(3), |app| !report_shown(app)) {
            failures.push("Hide details left the diagnostic report shown".into());
        }
    }
    let copied = |app: &App| {
        app.clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.text())
    };
    let before = copied(app);
    if let Err(error) = ps::click_popup_button(app, "Copy details") {
        failures.push(format!("notice Copy details: {error}"));
        return;
    }
    if !ps::wait(app, Duration::from_secs(3), |app| copied(app) != before) {
        failures.push("Copy details put nothing on the clipboard".into());
    } else {
        // The complete report: the title, the folder's whole path and the reason
        // (shown, or as the system reported it).
        let text = copied(app).unwrap_or_default();
        let path = locked.to_string_lossy();
        if !text.contains(TITLE)
            || !text.contains(path.as_ref())
            || !(text.contains(DENIED) || text.contains("Permission denied"))
        {
            failures.push(format!(
                "Copy details copied {text:?}: it lacks the title, the reason or the folder's whole path"
            ));
        } else if let Err(error) = crate::tools_support::notice_copy_starts_as_shown(app) {
            failures.push(error);
        }
    }
    if support::shown_notice(app).is_none() {
        failures.push("Copy details closed the notice".into());
    }
}

fn tree_error(
    app: &mut App,
    context: &mut Context,
    parent: &Path,
    locked: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    let disclosure = |app: &App| {
        app.rows(LEFT)
            .iter()
            .find(|row| row.name == LOCKED)
            .map(|row| row.tree_disclosure)
    };
    context.note(format!(
        "the unreadable folder's expander state before a click: {:?}",
        disclosure(app)
    ));
    if let Err(error) = click_expander(app, LOCKED) {
        failures.push(format!("inline branch: {error}"));
        return Ok(());
    }
    // A failed listing of a branch is not a navigation: no notice opens by itself.
    if !ps::wait(app, Duration::from_secs(10), |app| {
        disclosure(app) == Some(6) || support::shown_notice(app).is_some()
    }) {
        failures.push(format!(
            "expanding the unreadable folder never showed a failed branch (state {:?})",
            disclosure(app)
        ));
        return Ok(());
    }
    if let Some(notice) = support::shown_notice(app) {
        failures.push(format!(
            "expanding the unreadable folder opened a notice by itself: {notice:?}"
        ));
        support::dismiss_all(app);
    }
    app.screenshot(
        context,
        "branch-error",
        "An inline branch that cannot be listed",
    );
    let edge = app.window.get_left_list_left() + app.window.get_left_list_width();
    let icon = crate::inspect::find_all(
        app.slint(),
        i_slint_core::items::AccessibleRole::Button,
        "Folder listing error",
    )
    .into_iter()
    .find(|item| crate::inspect::center(item).x < edge);
    let Some(icon) = icon else {
        failures.push("the failed branch shows no Folder listing error icon".into());
        return Ok(());
    };
    input::click_at(app.slint(), crate::inspect::center(&icon));
    app.settle();
    if check_notice(
        app,
        context,
        "the failed branch's error icon",
        locked,
        "Folder listing error",
        None,
        Some("branch-error-notice"),
        failures,
    ) {
        close_notice(app, "branch error", "Escape", parent, None, failures);
    }
    if !ps::shown(app, LEFT).contains(&LOCKED.to_owned()) {
        failures.push("the failed branch's folder left the listing".into());
    }
    // Once readable, Retry lists the branch; then it is locked again for the rest.
    crate::fixtures::allow_read(locked)?;
    match click_expander(app, LOCKED) {
        Err(error) => failures.push(format!("the failed branch's Retry: {error}")),
        Ok(()) => {
            if !ps::wait(app, Duration::from_secs(10), |app| {
                app.rows(LEFT)
                    .iter()
                    .any(|row| row.name == "inside" && row.tree_depth == 1)
            }) {
                failures.push(format!(
                    "Retry on the readable folder did not list its branch: {:?}",
                    ps::shown(app, LEFT)
                ));
            } else if crate::inspect::find(
                app.slint(),
                i_slint_core::items::AccessibleRole::Button,
                "Folder listing error",
            )
            .is_some()
            {
                failures.push("the branch's error icon stayed after a successful Retry".into());
            }
            let _ = click_expander(app, LOCKED);
            ps::wait(app, Duration::from_secs(5), |app| {
                !app.rows(LEFT).iter().any(|row| row.name == "inside")
            });
        }
    }
    crate::fixtures::deny_read(locked)?;
    Ok(())
}

/// The same notice at touch density in a smaller window still shows the whole path.
fn touch(
    context: &mut Context,
    parent: &Path,
    locked: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let mut config = Config::new(parent, &context.work);
    config.density = UiDensity::Touch;
    config.width = 900;
    config.height = 640;
    let mut app = App::start(config);
    panel::select_row(&mut app, LEFT, LOCKED)?;
    input::key(app.slint(), Key::Return);
    if check_notice(
        &mut app,
        context,
        "Enter at touch density in a 900×640 window",
        locked,
        TITLE,
        Some(DENIED),
        Some("unreadable-touch"),
        failures,
    ) {
        close_notice(&mut app, "touch", "Close", parent, Some(LOCKED), failures);
    }
    blind_control(&mut app, locked, failures);
    app.shutdown()
}

/// The whole-path check must be able to fail: in a 480×260 window the long path cannot
/// fit the notice, so the check has to report it cut (the notice then scrolls, which
/// is right for so small a window).
fn blind_control(app: &mut App, locked: &Path, failures: &mut Vec<String>) {
    app.resize(480, 260);
    app.settle();
    if panel::focus_list(app, LEFT).is_err() {
        // The compact layout keeps the list; the cursor is still on the folder.
        support::dismiss_all(app);
    }
    input::key(app.slint(), Key::Return);
    if !ps::wait(app, Duration::from_secs(10), |app| {
        support::shown_notice(app).is_some()
    }) {
        failures.push("control: Enter in a 480×260 window opened no notice".into());
        return;
    }
    app.idle(Duration::from_millis(100));
    let wanted = locked.to_string_lossy().into_owned();
    match ps::dialog_texts(app)
        .into_iter()
        .find(|text| text.text == wanted)
    {
        None => failures.push("control: the small notice shows no path text".into()),
        Some(text) if !text.cut() => failures.push(format!(
            "control: in a 480×260 window the {}-character path ({:.0} px tall) cannot fit, yet the check finds it whole: the whole-path check is blind",
            wanted.chars().count(),
            text.full.3
        )),
        Some(_) => {}
    }
    support::dismiss_all(app);
    app.resize(900, 640);
    app.settle();
}
