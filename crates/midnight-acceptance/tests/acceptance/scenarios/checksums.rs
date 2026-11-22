//! Checksums (Ctrl+F8): calculate, save a manifest, verify it, replace it, and verify
//! the fixture's SHA-256, MD5 and SFV manifests including a mismatch and a missing file.
//! Copy is disabled while nothing is selected; with every row selected it copies exactly
//! the saved manifest.
use crate::{
    Context,
    app::{self, App, Config},
    encode, input, inspect, panel,
    scenarios::transfers::copy_tree,
    tools_support as tools,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow};
use std::{fs, rc::Rc};

const WIDTH: u32 = 1000;
const HEIGHT: u32 = 720;
const INPUTS: [&str; 4] = ["alpha.bin", "beta.bin", "delta empty.txt", "gamma.txt"];

fn texts(window: &MinimalSoftwareWindow) -> Vec<String> {
    inspect::labels(window.window(), AccessibleRole::Text)
}

/// Click the last visible button with this label (the footer, not a page tab).
fn click_last(window: &MinimalSoftwareWindow, label: &str) -> Result<(), String> {
    let item = inspect::find_all(window.window(), AccessibleRole::Button, label)
        .pop()
        .ok_or_else(|| format!("no {label:?} button; texts: {:?}", texts(window)))?;
    input::click_at(window.window(), inspect::center(&item));
    Ok(())
}

#[track_caller]
fn wait_text(
    app: &mut App,
    window: &Rc<MinimalSoftwareWindow>,
    what: &str,
    wanted: impl Fn(&str) -> bool,
) {
    let window = window.clone();
    app.until(what, move |_| {
        texts(&window).iter().any(|text| wanted(text))
    });
}

fn open_checksums(app: &mut App) -> Rc<MinimalSoftwareWindow> {
    let before = app.tool_windows().len();
    input::chord(app.slint(), &[Key::Control], Key::F8);
    app.until("the Checksums window", move |app| {
        app.tool_windows().len() > before
    });
    let window = app.tool_windows().pop().expect("checksum window");
    app::layout_tool(&window, WIDTH, HEIGHT);
    app.settle();
    window
}

fn verify_manifest(
    app: &mut App,
    context: &mut Context,
    manifest: &str,
    expect: &str,
    passed: &[&str],
    failed: &[&str],
) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, manifest)?;
    let window = open_checksums(app);
    // Ctrl+F8 on a manifest opens the Verify page with that file chosen.
    if inspect::find(
        window.window(),
        AccessibleRole::TextInput,
        "Checksum file to verify",
    )
    .is_none()
    {
        return Err(format!(
            "{manifest}: the window did not open on the Verify page; texts: {:?}",
            texts(&window)
        ));
    }
    click_last(&window, "Verify")?;
    let expected = expect.to_owned();
    wait_text(
        app,
        &window,
        &format!("{manifest} verification to finish"),
        move |text| text.starts_with(&expected),
    );
    let images = inspect::labels(window.window(), AccessibleRole::Image);
    for name in passed {
        if !images
            .iter()
            .any(|label| label == &format!("Passed: {name}"))
        {
            return Err(format!(
                "{manifest}: {name} did not pass; results {images:?}"
            ));
        }
    }
    for name in failed {
        if !images
            .iter()
            .any(|label| label == &format!("Failed: {name}"))
        {
            return Err(format!(
                "{manifest}: {name} did not fail; results {images:?}"
            ));
        }
    }
    app.screenshot_window(
        context,
        &window,
        &format!("verify-{manifest}"),
        &format!("Verify {manifest}"),
    );
    input::key(window.window(), Key::Escape);
    app.until("the Checksums window to close", |app| {
        app.tool_windows().is_empty()
    });
    Ok(())
}

/// The footer's Copy action, whose caption names the shown algorithm.
fn copy_action(app: &mut App, window: &MinimalSoftwareWindow) -> Result<tools::Action, String> {
    let footer = tools::footer_actions(app, window.window(), "Close")?;
    footer
        .all()
        .find(|action| action.label.starts_with("Copy") && action.label != "Copy details")
        .cloned()
        .ok_or_else(|| format!("the footer has no Copy: {:?}", footer.labels()))
}

pub fn calculate_save_verify(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("checksums");
    copy_tree(&context.fixture.path("checksums"), &folder)?;
    let mut config = Config::new(&folder, &context.work);
    // Copy reaches the in-process clipboard.
    config.clipboard = true;
    let mut app = App::start(config);
    let pane = PaneId::Left;

    // Mark the four data files with Insert, then Ctrl+F8.
    for name in INPUTS {
        panel::select_row(&mut app, pane, name)?;
        input::key(app.slint(), Key::Insert);
    }
    app.settle();
    let marked: Vec<String> = app
        .rows(pane)
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect();
    if marked.len() != INPUTS.len() {
        return Err(format!("Insert marked {marked:?}"));
    }
    let window = open_checksums(&mut app);
    app.screenshot_window(
        context,
        &window,
        "checksums-open",
        "Checksums window before Calculate",
    );
    click_last(&window, "Calculate")?;
    wait_text(&mut app, &window, "the calculation to finish", |text| {
        text.starts_with("Completed · 4 calculated")
    });
    let rows = inspect::labels(window.window(), AccessibleRole::ListItem);
    let mut expected_manifest = Vec::new();
    for name in INPUTS {
        let data = fs::read(folder.join(name)).map_err(|error| error.to_string())?;
        let digest = encode::hex(&encode::sha256(&data));
        if !rows
            .iter()
            .any(|row| row.starts_with(&format!("{name} · ")) && row.contains(&digest))
        {
            return Err(format!(
                "no result row for {name} with SHA-256 {digest}; rows {rows:#?}"
            ));
        }
        expected_manifest.push(format!("{digest} *{name}"));
    }
    app.screenshot_window(
        context,
        &window,
        "checksums-calculated",
        "Four SHA-256 checksums calculated",
    );
    // Nothing is selected yet: Copy is there, disabled, and says why.
    let copy = copy_action(&mut app, &window)?;
    if copy.label != "Copy SHA-256"
        || copy.enabled
        || !copy.description.starts_with("Nothing to copy: ")
    {
        return Err(format!(
            "before any selection the footer offers {:?} (enabled {}, hint {:?})",
            copy.label, copy.enabled, copy.description
        ));
    }

    // Save manifest…: the scripted chooser accepts the suggested checksums.sha256.
    let saves = app.manifests.len();
    click_last(&window, "Save manifest…")?;
    app.until("the manifest save", move |app| app.manifests.len() > saves);
    if let Some(Err(error)) = app.manifests.last() {
        return Err(format!("saving the manifest failed: {error}"));
    }
    // The save went through the (scripted) file chooser, suggested in the panel folder.
    let request = app
        .picker
        .requests()
        .pop()
        .ok_or("Save manifest… asked no file chooser")?;
    if request.initial_directory.as_deref() != Some(folder.as_path()) {
        return Err(format!(
            "the chooser opened in {:?}, not the panel folder",
            request.initial_directory
        ));
    }
    let saved =
        fs::read_to_string(folder.join("checksums.sha256")).map_err(|error| error.to_string())?;
    let mut lines: Vec<&str> = saved.lines().collect();
    lines.sort_unstable();
    expected_manifest.sort();
    if lines != expected_manifest {
        return Err(format!(
            "checksums.sha256 is {saved:?}, expected {expected_manifest:?}"
        ));
    }

    // Copy copies the selected rows as the checksum file Save manifest… writes: with
    // every row selected (Ctrl+A in the results), exactly the saved file.
    let row = tools::list_rows(&window, "Checksum results")
        .into_iter()
        .next()
        .ok_or("no result row to click")?;
    let (x, y, w, h) = row.rect;
    input::click_at(
        window.window(),
        slint::LogicalPosition::new(x + w.min(240.0) / 2.0, y + h / 2.0),
    );
    app.settle();
    input::chord(window.window(), &[Key::Control], "a");
    app.settle();
    let copy = copy_action(&mut app, &window)?;
    if copy.label != "Copy 4 SHA-256" || !copy.enabled {
        return Err(format!(
            "with every row selected the footer offers {:?} (enabled {})",
            copy.label, copy.enabled
        ));
    }
    let before = tools::clipboard_writes(&app);
    input::click_at(
        window.window(),
        slint::LogicalPosition::new(
            copy.rect.0 + copy.rect.2 / 2.0,
            copy.rect.1 + copy.rect.3 / 2.0,
        ),
    );
    let copied = tools::wait_copied(&mut app, before, "Copy 4 SHA-256")?;
    if copied != saved {
        return Err(format!(
            "Copy 4 SHA-256 copied {copied:?}, not the saved checksums.sha256 {saved:?}"
        ));
    }

    // Saving again asks before replacing; Replace writes it again.
    let saves = app.manifests.len();
    click_last(&window, "Save manifest…")?;
    wait_text(&mut app, &window, "the Replace prompt", |text| {
        text == "checksums.sha256 already exists. Replace it?"
    });
    app.screenshot_window(
        context,
        &window,
        "checksums-replace-prompt",
        "Replace prompt for an existing manifest",
    );
    click_last(&window, "Replace")?;
    app.until("the replacing save", move |app| {
        app.manifests[saves..]
            .iter()
            .filter(|result| result.is_ok())
            .count()
            == 1
    });

    // The Verify page, then its Verify button: the field already names the saved
    // manifest. On the Calculate page the page tab is the only "Verify".
    click_last(&window, "Verify")?;
    app.settle();
    click_last(&window, "Verify")?;
    wait_text(&mut app, &window, "the verification", |text| {
        text.starts_with("Completed · 4 verified · 0 mismatched · 0 missing")
    });
    for name in INPUTS {
        if inspect::find(
            window.window(),
            AccessibleRole::Image,
            &format!("Passed: {name}"),
        )
        .is_none()
        {
            return Err(format!("{name} is not reported as passed"));
        }
    }
    app.screenshot_window(
        context,
        &window,
        "checksums-verified",
        "The saved manifest verified",
    );
    input::key(window.window(), Key::Escape);
    app.until("the Checksums window to close", |app| {
        app.tool_windows().is_empty()
    });
    if !app.checksum_errors.is_empty() {
        return Err(format!("checksum errors: {:?}", app.checksum_errors));
    }

    // The fixture's own manifests.
    panel::focus_list(&mut app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    verify_manifest(
        &mut app,
        context,
        "all.md5",
        "Completed · 4 verified",
        &INPUTS,
        &[],
    )?;
    verify_manifest(
        &mut app,
        context,
        "all.sfv",
        "Completed · 4 verified",
        &INPUTS,
        &[],
    )?;
    verify_manifest(
        &mut app,
        context,
        "mismatch.sha256",
        "Completed with differences",
        &["alpha.bin", "beta.bin"],
        &["gamma.txt"],
    )?;
    verify_manifest(
        &mut app,
        context,
        "missing.sha256",
        "Completed with differences",
        &["alpha.bin"],
        &["missing-file.bin"],
    )?;
    context.note("calculated 4 SHA-256 values, saved, replaced and verified checksums.sha256, and verified all.md5, all.sfv, mismatch.sha256 and missing.sha256");
    app.shutdown()
}
