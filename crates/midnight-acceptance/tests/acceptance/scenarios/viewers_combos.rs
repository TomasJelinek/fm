//! Viewers in combination with the rest of the application:
//!
//! - F3 on a result in the Find window, and F3 in its results fed to a panel, where
//!   Space and Backspace follow the results' order; Compare files takes the focused
//!   result as File A, and Backspace leaves the list.
//! - F3 on a file shown in an expanded inline tree branch: Space follows the visible
//!   file order through the branch and on to the folder's own files.
//! - Four viewer windows at once, each keeping its own file; a fifth is refused with the
//!   reason; closing one keeps the others and lets another open.
//! - A theme previewed in Options recolours an open viewer at once (background and
//!   syntax colours) and Cancel restores it.
//! - A viewer moved to a display of twice the scale: the same lines, twice as tall.
use crate::{
    Context,
    app::{App, Config},
    files_support as files, input, options_support as options, panel,
    scenarios::find_support as find,
    tools_support as tools, viewers_stage,
    viewers_support::{self as support, Failures, Viewing, generated, probe},
};
use midnight_core::{PaneId, PreferenceTheme};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(15 * 60);
const OPEN_LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_secs(15);

pub fn combinations(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn prepare(context: &Context) -> Result<std::path::PathBuf, String> {
    let root = context.work.join("combo");
    for (name, text) in [
        ("find/a.txt", "Alpha text.\n"),
        ("find/b.txt", "Bravo text.\n"),
        ("find/sub/c.txt", "Charlie text.\n"),
        ("find/d.md", "# Delta\n"),
        ("tree/top.txt", "Top of the tree.\n"),
        ("tree/zeta.txt", "Zeta at the end.\n"),
        ("tree/one/two/deep.txt", "Deep in the branch.\n"),
    ] {
        generated::write(&root, name, text)?;
    }
    for index in 1..=5 {
        generated::write(
            &root,
            &format!("multi/w{index}.txt"),
            format!("Window {index} of five.\n"),
        )?;
    }
    generated::write(&root, "theme/sample.rs", generated::rust(200))?;
    generated::write(
        &root,
        "compact/notes.txt",
        "Notes in a small window.\n".repeat(80),
    )?;
    generated::write(&root, "compact/sample.rs", generated::rust(120))?;
    generated::write(&root, "compact/doc.md", generated::markdown(3))?;
    generated::write(&root, "compact/pages.pdf", generated::pdf(3))?;
    generated::write(
        &root,
        "compact/picture.png",
        crate::encode::png(400, 300, generated::picture),
    )?;
    generated::write(&root, "compact/page.html", generated::html(40))?;
    Ok(root)
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = prepare(context)?;
    let mut config = Config::new(&root, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let failures = Failures::default();
    if let Some(reason) = probe::html_unavailable() {
        failures.skip("small viewer windows: page.html", &reason);
    }
    // A section records the checks this machine cannot make in `failures` itself.
    type Section = fn(&mut App, &mut Context, &Failures) -> Result<(), String>;
    let sections: [(&str, Section, &str); 6] = [
        ("Find results and a fed list", find_results, "find"),
        ("an inline tree branch", tree_branch, "tree"),
        ("four viewer windows", several_windows, "multi"),
        ("a theme previewed in Options", theme_preview, "theme"),
        ("a viewer at twice the scale", scale, "theme"),
        ("small viewer windows", compact, "compact"),
    ];
    for (what, section, folder) in sections {
        let result = panel::enter(&mut app, PaneId::Left, folder)
            .and_then(|_| section(&mut app, context, &failures));
        failures.check(what, result);
        files::dismiss_all(&mut app);
        // Back to the root for the next section, whatever the section left.
        let wanted = root.clone();
        for _ in 0..4 {
            if app.location(PaneId::Left).as_ref() == Some(&wanted) {
                break;
            }
            let _ = panel::focus_list(&mut app, PaneId::Left);
            input::key(app.slint(), Key::Backspace);
            app.idle(Duration::from_millis(300));
        }
    }
    app.shutdown()?;
    failures.finish(context)
}

/// Close a viewer and give the main window back its activation.
fn close(app: &mut App, viewing: Viewing) -> Result<(), String> {
    viewing.close(app)
}

fn wait_text(app: &mut App, viewing: &Viewing, text: &str) -> Result<(), String> {
    viewing.wait(app, text, STEP, |window| {
        window.get_accessible_text().contains(text)
    })
}

/// F3 in the Find window on a result, then the results fed to the panel: F3 there,
/// Space and Backspace through the results, Compare files from the list, Backspace out.
fn find_results(app: &mut App, context: &mut Context, _: &Failures) -> Result<(), String> {
    let window = find::open_on(app, PaneId::Left)?;
    let (text, rows) = find::search(
        app,
        &window,
        find::Query {
            masks: "*.txt",
            ..find::BASE
        },
    )?;
    let names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    if !text.starts_with("Completed") || names.len() != 3 {
        return Err(format!("*.txt found {names:?} ({text:?})"));
    }
    find::select_result(app, &window, "b.txt", None)?;
    let before = support::viewers_opened(app);
    input::key(window.window(), Key::F3);
    let viewing = support::next_viewer(app, before, OPEN_LIMIT)?;
    let shown = wait_text(app, &viewing, "Bravo text.");
    app.screenshot_window(
        context,
        &viewing.software,
        "find-result-f3",
        "F3 on the result b.txt in the Find window",
    );
    close(app, viewing)?;
    shown?;
    tools::click_last(window.window(), "Feed to panel")?;
    app.until("the results list to open", |app| {
        find::feed_view(app, PaneId::Left).open
    });
    let watched = window.clone();
    app.until("Feed to panel to close the Find window", move |_| {
        !watched.window().is_visible()
    });
    panel::wait_input(app);
    let order = find::feed_names(app, PaneId::Left);
    // The cursor to the first result, then F3.
    input::key(app.slint(), Key::Home);
    app.settle();
    let first = find::feed_cursor(app, PaneId::Left).ok_or("the fed list has no cursor")?;
    let texts = |name: &str| match name {
        "a.txt" => "Alpha text.",
        "b.txt" => "Bravo text.",
        "c.txt" => "Charlie text.",
        _ => "",
    };
    let before = support::viewers_opened(app);
    input::key(app.slint(), Key::F3);
    let viewing = support::next_viewer(app, before, OPEN_LIMIT)?;
    let walked = (|| {
        wait_text(app, &viewing, texts(&first))?;
        viewing.click_content(app);
        let index = order
            .iter()
            .position(|name| *name == first)
            .ok_or("the cursor's row is not in the list")?;
        let next = &order[(index + 1) % order.len()];
        input::key(viewing.slint(), Key::Space);
        wait_text(app, &viewing, texts(next))?;
        app.screenshot_window(
            context,
            &viewing.software,
            "feed-space",
            &format!("Space in a viewer opened from the fed list: {next}"),
        );
        input::key(viewing.slint(), Key::Backspace);
        wait_text(app, &viewing, texts(&first))?;
        context.note(format!(
            "Fed list {order:?}: F3 on {first}, Space to {next}, Backspace back"
        ));
        Ok::<(), String>(())
    })();
    close(app, viewing)?;
    walked?;
    // Compare files takes the fed list's focused result (here one in a subfolder, which
    // the folder under the list does not show) as File A, and asks for File B: the other
    // panel has no file of that name.
    let folder = app
        .location(PaneId::Left)
        .ok_or("the panel under the fed list has no folder")?;
    let nested = folder.join("sub").join("c.txt");
    input::key(app.slint(), Key::Home);
    app.settle();
    for _ in 0..order.len() {
        if find::feed_cursor(app, PaneId::Left).as_deref() == Some("c.txt") {
            break;
        }
        input::key(app.slint(), Key::DownArrow);
        app.settle();
    }
    if find::feed_cursor(app, PaneId::Left).as_deref() != Some("c.txt") {
        return Err(format!(
            "the fed list {order:?} has no c.txt to put the cursor on"
        ));
    }
    let before = support::plugins(app)?.compared.borrow().len();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let comparator = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .and_then(|compared| compared.window.upgrade())
        .ok_or_else(|| {
            format!(
                "Ctrl+Shift+C in a fed list opened no comparator; notice {:?}",
                app.notice()
            )
        })?;
    comparator
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let wanted = nested.to_string_lossy().into_owned();
    let deadline = std::time::Instant::now() + STEP;
    let form = loop {
        app.settle();
        support::paint_tool_windows(app);
        if comparator.get_choosing_files() {
            break Ok(());
        }
        if std::time::Instant::now() >= deadline {
            break Err(format!(
                "Ctrl+Shift+C in a fed list: no File A / File B form; status {:?}",
                comparator.get_status()
            ));
        }
        app.idle(Duration::from_millis(10));
    }
    .and_then(|()| {
        let (file_a, file_b) = (comparator.get_file_a(), comparator.get_file_b());
        if file_a.as_str() == wanted && file_b.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Ctrl+Shift+C in a fed list: File A {file_a:?} and File B {file_b:?}, expected File A {wanted:?}"
            ))
        }
    });
    input::click_button(comparator.window(), "Cancel")?;
    let deadline = std::time::Instant::now() + STEP;
    while support::software_window(app, comparator.window()).is_some() {
        if std::time::Instant::now() >= deadline {
            return Err("Cancel did not close the File A / File B form".into());
        }
        app.idle(Duration::from_millis(10));
    }
    drop(comparator);
    app.slint()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    app.settle();
    form?;
    panel::wait_input(app);
    input::key(app.slint(), Key::Backspace);
    app.until("Backspace to leave the fed list", |app| {
        !find::feed_view(app, PaneId::Left).open
    });
    panel::leave(app, PaneId::Left)?;
    Ok(())
}

/// F3 in an expanded inline branch; Space follows the visible files across it.
/// Inline branches list folders: with `one` expanded (showing `two`), F3 on the
/// folder's own files works, and Space and Backspace follow the visible file order.
fn tree_branch(app: &mut App, context: &mut Context, _: &Failures) -> Result<(), String> {
    files::toggle_tree(app, "one")?;
    files::wait_tree_depth(app, "two", Some(1));
    panel::select_row(app, PaneId::Left, "top.txt")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let walked = (|| {
        wait_text(app, &viewing, "Top of the tree.")?;
        viewing.click_content(app);
        input::key(viewing.slint(), Key::Space);
        wait_text(app, &viewing, "Zeta at the end.")?;
        app.screenshot_window(
            context,
            &viewing.software,
            "tree-space",
            "Space from top.txt while the branch one/two is expanded: the next file",
        );
        input::key(viewing.slint(), Key::Backspace);
        wait_text(app, &viewing, "Top of the tree.")?;
        Ok::<(), String>(())
    })();
    close(app, viewing)?;
    walked?;
    files::toggle_tree(app, "one")?;
    files::wait_tree_depth(app, "two", None);
    panel::leave(app, PaneId::Left)?;
    Ok(())
}

/// Four viewers at once; a fifth refused; closing one keeps the others.
fn several_windows(app: &mut App, context: &mut Context, _: &Failures) -> Result<(), String> {
    let mut open: Vec<(usize, Viewing)> = Vec::new();
    let result = (|| {
        for index in 1..=4 {
            panel::select_row(app, PaneId::Left, &format!("w{index}.txt"))?;
            let viewing = support::view(app, false, OPEN_LIMIT)?;
            wait_text(app, &viewing, &format!("Window {index} of five."))?;
            app.slint()
                .dispatch_event(WindowEvent::WindowActiveChanged(true));
            open.push((index, viewing));
        }
        panel::select_row(app, PaneId::Left, "w5.txt")?;
        let refused = support::view(app, false, OPEN_LIMIT);
        let notice = app.notice();
        if refused.is_ok() {
            return Err("a fifth viewer opened".into());
        }
        if !notice
            .as_ref()
            .is_some_and(|(_, body)| body.contains("maximum four windows"))
        {
            return Err(format!(
                "the fifth viewer was refused without the reason: notice {notice:?}"
            ));
        }
        app.screenshot(
            context,
            "fifth-viewer",
            "F3 with four viewers open: refused with the reason",
        );
        input::key(app.slint(), Key::Escape);
        app.settle();
        for (index, viewing) in &open {
            wait_text(app, viewing, &format!("Window {index} of five."))?;
        }
        // Close the second; the others keep their files.
        let (_, second) = open.remove(1);
        close(app, second)?;
        for (index, viewing) in &open {
            if !viewing.window.window().is_visible() {
                return Err(format!("closing the second viewer closed viewer {index}"));
            }
            wait_text(app, viewing, &format!("Window {index} of five."))?;
        }
        panel::select_row(app, PaneId::Left, "w5.txt")?;
        let fifth = support::view(app, false, OPEN_LIMIT)?;
        wait_text(app, &fifth, "Window 5 of five.")?;
        open.push((5, fifth));
        Ok(())
    })();
    for (_, viewing) in open {
        let _ = close(app, viewing);
    }
    result?;
    panel::leave(app, PaneId::Left)?;
    Ok(())
}

/// A text colour covering at least this many pixels shows its solid core, so its
/// contrast is checked strictly; smaller groups are noted.
const CORE_PIXELS: usize = 300;

/// A page's colour and the text colours on it: (core colour, pixels of its group).
type Colours = ([u8; 3], Vec<([u8; 3], usize)>);

/// The viewer's background and its text colours right of the line-number gutter, each
/// measured at its glyphs' solid cores (anti-aliased edges are blends towards the
/// background), with the pixels of its group.
fn colours(app: &mut App, viewing: &Viewing) -> Result<Colours, String> {
    let canvas = probe::still_canvas(app, viewing, STEP)?;
    let canvas = probe::right_of_gutter(viewing, &canvas, 6);
    let background = probe::dominant(&canvas);
    Ok((background, probe::glyph_cores(&canvas, background, 24)))
}

/// A theme previewed in Options recolours the open viewer; Cancel restores it.
fn theme_preview(app: &mut App, context: &mut Context, _: &Failures) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, "sample.rs")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let result = (|| {
        wait_text(app, &viewing, "fn function_0")?;
        let (dark, dark_glyphs) = colours(app, &viewing)?;
        app.set_host_dark(false);
        viewing.wait(
            app,
            "the viewer to adopt the host's light theme",
            STEP,
            |_| probe::dominant(&viewing.canvas()) != dark,
        )?;
        let (original_light, _) = colours(app, &viewing)?;
        app.slint()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        options::open(app)?;
        options::page(app, "Appearance")?;
        options::choose(app, "Light theme", "Light")?;
        viewing.wait(
            app,
            "the viewer to follow the previewed theme",
            STEP,
            |_| {
                let current = probe::dominant(&viewing.canvas());
                current != original_light && probe::luminance(current) > 0.6
            },
        )?;
        let (light, light_glyphs) = colours(app, &viewing)?;
        app.screenshot_window(
            context,
            &viewing.software,
            "theme-preview-light",
            "The open viewer while Options previews the Light theme",
        );
        let colour_set = |glyphs: &[([u8; 3], usize)]| {
            glyphs.iter().map(|(colour, _)| *colour).collect::<Vec<_>>()
        };
        if colour_set(&light_glyphs) == colour_set(&dark_glyphs) {
            return Err("the syntax colours did not change with the theme".into());
        }
        // The product keeps syntax colours at 5:1 against the page in the Light theme.
        let minimum = probe::syntax_minimum(PreferenceTheme::Light);
        let below = |strict: bool| -> Vec<String> {
            light_glyphs
                .iter()
                .filter(|(colour, pixels)| {
                    probe::contrast(*colour, light) < minimum && (*pixels >= CORE_PIXELS) == strict
                })
                .map(|(colour, pixels)| {
                    format!(
                        "{} ({pixels} px) {:.2}:1",
                        probe::hex(*colour),
                        probe::contrast(*colour, light)
                    )
                })
                .collect()
        };
        let (dim, faint) = (below(true), below(false));
        options::cancel(app)?;
        viewing.wait(app, "Cancel to restore the viewer's theme", STEP, |_| {
            probe::dominant(&viewing.canvas()) == original_light
        })?;
        app.set_host_dark(true);
        viewing.wait(app, "the viewer to return to its dark theme", STEP, |_| {
            probe::dominant(&viewing.canvas()) == dark
        })?;
        context.note(format!(
            "Theme preview: background {} → {} → {}; text colours {} → {}; Light colours of \
             fewer than {CORE_PIXELS} pixels below {minimum}:1 (cores may be blends): {faint:?}",
            probe::hex(dark),
            probe::hex(light),
            probe::hex(dark),
            dark_glyphs.len(),
            light_glyphs.len()
        ));
        if !dim.is_empty() {
            return Err(format!(
                "in the previewed Light theme these text colours stay below {minimum}:1 against \
                 the page {}: {dim:?}",
                probe::hex(light)
            ));
        }
        Ok(())
    })();
    close(app, viewing)?;
    result
}

/// The viewer moved to a display of twice the scale (same logical size): the same
/// first lines, rendered twice as tall, and no error.
fn scale(app: &mut App, context: &mut Context, _: &Failures) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, "sample.rs")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let result = (|| {
        wait_text(app, &viewing, "fn function_0")?;
        let before = probe::still_canvas(app, &viewing, STEP)?;
        let rows_before = ink_bands(&before);
        let size = viewing.slint().size();
        viewing
            .slint()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
        viewing
            .software
            .set_size(slint::PhysicalSize::new(size.width * 2, size.height * 2));
        viewing.slint().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(size.width as f32, size.height as f32),
        });
        let after = probe::still_canvas(app, &viewing, STEP)?;
        let rows_after = ink_bands(&after);
        app.screenshot_window(
            context,
            &viewing.software,
            "scale-2",
            "sample.rs after the window moved to a display of twice the scale",
        );
        if let Some(error) = probe::status_error(&viewing.status()) {
            return Err(format!("at scale 2 the status reports {error:?}"));
        }
        let height = |bands: &[(usize, usize)]| {
            let mut heights: Vec<usize> = bands.iter().map(|(top, bottom)| bottom - top).collect();
            heights.sort_unstable();
            heights.get(heights.len() / 2).copied().unwrap_or(0)
        };
        let (old, new) = (height(&rows_before), height(&rows_after));
        context.note(format!(
            "Scale 2: {} text rows of {old} px became {} of {new} px",
            rows_before.len(),
            rows_after.len()
        ));
        if rows_after.is_empty() || !(old * 3 / 2..=old * 5 / 2).contains(&new) {
            return Err(format!(
                "text rows of {old} px are {new} px tall at twice the scale ({} rows)",
                rows_after.len()
            ));
        }
        let ratio = rows_after.len() as f32 / rows_before.len().max(1) as f32;
        if !(0.8..=1.25).contains(&ratio) {
            return Err(format!(
                "{} text rows at scale 1 but {} at scale 2",
                rows_before.len(),
                rows_after.len()
            ));
        }
        Ok(())
    })();
    close(app, viewing)?;
    panel::leave(app, PaneId::Left)?;
    result
}

/// Every viewer kind in a small window (480 x 320, then the 320 x 200 minimum): the
/// controls that kind needs stay inside the window, and the UI review's layout checks
/// run on each (their findings are flags, not failures).
fn compact(app: &mut App, context: &mut Context, failures: &Failures) -> Result<(), String> {
    use i_slint_core::items::AccessibleRole;
    let kinds: [(&str, &[(AccessibleRole, &str)]); 6] = [
        (
            "notes.txt",
            &[
                (AccessibleRole::Button, "Find"),
                (AccessibleRole::Combobox, "Viewer mode"),
            ],
        ),
        (
            "sample.rs",
            &[
                (AccessibleRole::Checkbox, "Source line numbers"),
                (AccessibleRole::Combobox, "Viewer mode"),
            ],
        ),
        (
            "doc.md",
            &[
                (AccessibleRole::Combobox, "Markdown presentation"),
                (AccessibleRole::Combobox, "Viewer mode"),
            ],
        ),
        (
            "pages.pdf",
            &[
                (AccessibleRole::Button, "Next document page (Ctrl+PageDown)"),
                (AccessibleRole::Button, "Fit image to window"),
            ],
        ),
        (
            "picture.png",
            &[
                (AccessibleRole::Button, "Fit image to window"),
                (AccessibleRole::Button, "Zoom image in"),
            ],
        ),
        ("page.html", &[(AccessibleRole::Button, "Zoom HTML in")]),
    ];
    let mut problems = Vec::new();
    for (file, controls) in kinds {
        if file.ends_with(".html")
            && let Some(reason) = probe::html_unavailable()
        {
            context.note(format!("{file} in small windows not checked: {reason}"));
            continue;
        }
        panel::select_row(app, PaneId::Left, file)?;
        let viewing = support::view(app, false, OPEN_LIMIT)?;
        if let Err(error) = viewing.result.clone() {
            close(app, viewing)?;
            match support::browser_refused(&error) {
                Some(reason) if file.ends_with(".html") => failures.skip(
                    &format!("small viewer windows: {file}"),
                    &format!("{reason}: {}", support::clip(&error, 160)),
                ),
                _ => problems.push(format!("{file}: the viewer reported {error:?}")),
            }
            continue;
        }
        for (width, height) in [(480, 320), (320, 200)] {
            crate::app::layout_tool(&viewing.software, width, height);
            probe::pause(app, Duration::from_millis(400));
            let _ = probe::still_canvas(app, &viewing, STEP);
            let shot = app.screenshot_window(
                context,
                &viewing.software,
                &format!("small-{}-{width}x{height}", file.replace('.', "-")),
                &format!("{file} in a {width} x {height} viewer window"),
            );
            for finding in probe::review(context, viewing.slint(), &shot) {
                context.note(format!("{file} at {width} x {height}: {finding}"));
            }
            for (role, label) in controls {
                // Placed, not drawn: a WrappingCheckBox ("Source line numbers") draws
                // its box and caption beside an accessible check box under an
                // opacity-0 layer, which `visible_rect` would drop.
                let shown = crate::inspect::find(viewing.slint(), *role, label)
                    .and_then(|item| crate::inspect::placed_rect(&item))
                    .is_some_and(|(x, y, w, h)| {
                        x >= -0.5
                            && y >= -0.5
                            && x + w <= width as f32 + 0.5
                            && y + h <= height as f32 + 0.5
                    });
                if !shown {
                    problems.push(format!(
                        "{file} at {width} x {height}: {role:?} {label:?} is not inside the window"
                    ));
                }
            }
        }
        close(app, viewing)?;
    }
    panel::leave(app, PaneId::Left)?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The canvas's horizontal bands of ink (text rows): (top, bottom) pixel rows.
fn ink_bands(canvas: &support::Canvas) -> Vec<(usize, usize)> {
    let background = probe::dominant(canvas);
    let mut bands = Vec::new();
    let mut start = None;
    for y in 0..canvas.height {
        let ink = (0..canvas.width).any(|x| canvas.at(x, y) != background);
        match (ink, start) {
            (true, None) => start = Some(y),
            (false, Some(top)) => {
                bands.push((top, y));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(top) = start {
        bands.push((top, canvas.height));
    }
    bands
}
