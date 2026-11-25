//! Real F7 and F2 folder forms, including what fits and what typing immediately changes.
use crate::{
    Context,
    app::{App, Config},
    files_support as files, input, inspect, panel, tools_support as controls,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{OperationState, PaneId};
use slint::platform::Key;
use std::{fs, path::Path};

pub fn folder_dialogs(context: &mut Context) -> Result<(), String> {
    let folder = files::scratch(context, "folder-dialogs")?
        .join("Monthly project deliveries")
        .join("Release candidates");
    fs::create_dir_all(&folder).map_err(|error| files::io_error(&folder, error))?;
    let mut app = App::start(Config::new(&folder, &context.work));
    let pane = PaneId::Left;
    panel::focus_list(&mut app, pane)?;
    let original = "Výkres.rev.2";
    let renamed = "Návrh.2026";
    for (key, title, field, action, name) in [
        (
            Key::F7,
            "Create directory",
            "Directory name",
            "Create",
            original,
        ),
        (Key::F2, "Rename folder", "New name", "Rename", renamed),
    ] {
        for (width, height, size) in [(1024, 720, "desktop"), (420, 320, "compact")] {
            app.resize(width, height);
            if key == Key::F2 {
                panel::select_row(&mut app, pane, original)?;
            }
            panel::wait_input(&mut app);
            let start = app.operations.len();
            input::key(app.slint(), key);
            files::wait_operation_dialog(&mut app, title);
            app.screenshot(
                context,
                &format!("{}-{size}", action.to_lowercase()),
                &format!("{title} at {width} × {height}"),
            );
            check_form(&app, title, field, action, &folder)?;

            // No click or Ctrl+A: the field must own focus and a dotted folder
            // name must be selected through the last character on opening F2.
            input::type_text(app.slint(), name);
            if size == "desktop" {
                input::key(app.slint(), Key::Escape);
                app.until("the cancelled folder dialog to close", |app| {
                    app.window.get_operation_token().is_empty()
                });
                let cancelled = files::wait_operation(&mut app, start);
                if cancelled.state != OperationState::Cancelled || folder.join(name).exists() {
                    return Err(format!("Escape in {title} applied the edit"));
                }
                continue;
            }
            input::key(app.slint(), Key::Return);
            files::operation_ok(&files::wait_operation(&mut app, start), title)?;
            if !folder.join(name).is_dir() {
                return Err(format!(
                    "{title} did not produce the typed directory name {name:?}"
                ));
            }
            files::wait_row(&mut app, pane, name, true);
            panel::wait_input(&mut app);
        }
        if key == Key::F7 {
            let marker = folder.join(original).join("contents.txt");
            fs::write(&marker, b"keep").map_err(|error| files::io_error(&marker, error))?;
        }
    }
    let marker = folder.join(renamed).join("contents.txt");
    if folder.join(original).exists()
        || fs::read(&marker).map_err(|error| files::io_error(&marker, error))? != b"keep"
    {
        return Err("the renamed folder or its contents differ from the expected result".into());
    }
    app.shutdown()
}

fn check_form(
    app: &App,
    title: &str,
    field: &str,
    action: &str,
    parent: &Path,
) -> Result<(), String> {
    let window = app.slint();
    if !inspect::visible_texts(window)
        .iter()
        .any(|text| text == title)
    {
        return Err(format!("the folder dialog heading {title:?} is missing"));
    }
    for (role, label) in [
        (AccessibleRole::TextInput, field),
        (AccessibleRole::Button, action),
        (AccessibleRole::Button, "Cancel"),
    ] {
        let item = controls::control(window, role, label)?;
        let Some((_, _, width, height)) = inspect::visible_rect(&item) else {
            return Err(format!("{title}: {label} is outside the visible form"));
        };
        let (_, _, full_width, full_height) = inspect::rect(&item);
        if width + 1.0 < full_width || height + 1.0 < full_height {
            return Err(format!("{title}: {label} is clipped"));
        }
    }
    let popup = inspect::roots(window).pop().ok_or("no folder dialog")?;
    let path = parent.to_string_lossy().replace('\\', "/");
    let mut path_available = false;
    inspect::walk(&popup, &mut |item| {
        if !path_available && inspect::visible_rect(item).is_some() {
            path_available = [
                inspect::label(item).unwrap_or_default(),
                controls::description(item),
                controls::value(item),
            ]
            .iter()
            .any(|text| text.replace('\\', "/").contains(&path));
        }
    });
    if !path_available {
        return Err(format!(
            "{title} does not expose its full location {path:?}"
        ));
    }
    let size = window.size().to_logical(window.scale_factor());
    let findings = inspect::check_layout(window, size.width, size.height);
    if !findings.is_empty() {
        let details: Vec<_> = findings.into_iter().map(|finding| finding.detail).collect();
        return Err(format!("{title} layout: {}", details.join(", ")));
    }
    Ok(())
}
