//! The Copy/Move dialog itself: what F5 and F6 show (the Links choice on a fixed local
//! folder: "Keep links as links" of its two entries, no reason or warning), a destination
//! typed as a missing folder, a relative path or nothing (refused in the dialog by Copy
//! and Move alike, nothing copied), Escape, Move's Cancel button, the order in which Tab
//! visits the dialog's controls, "Preserve times and attributes" offered for files and folders, the
//! verification chosen in Options › General (full SHA-256, or size and samples) and what
//! the job then verifies, and many small files in one job.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, TransferSpec, compare_trees, io_error, tree_facts},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
    scenarios::transfers::copy_tree,
    tools_support as tools,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, TransferVerification};
use slint::platform::Key;
use std::{fs, time::Instant};

const LEFT: PaneId = PaneId::Left;
const FULL: &str = "Verification: full SHA-256.";
const QUICK: &str = "Quick verification: size + first/middle/last 64 KiB";
const MOVE_POLICY: &str = "Same-volume items are renamed without reading file contents.";
const MISSING: &str = "Enter the full path of an existing destination folder.";
/// What the dialog says under an empty destination field when Enter is pressed.
const TYPE_FIRST: &str = "Type the destination folder first.";
const VERIFY_OPTION: &str = "Verify entire copied file with SHA-256 (recommended)";
const PRESERVE: &str = support::PRESERVE_OPTION;

/// Open the F5/F6 dialog on the focused panel.
fn open_dialog(app: &mut App, key: Key) {
    panel::wait_input(app);
    input::key(app.slint(), key);
    app.until("the Copy/Move dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
}

fn close_dialog(app: &mut App) {
    input::key(app.slint(), Key::Escape);
    app.until("the Copy/Move dialog to close", |app| {
        app.window.get_copy_draft_token().is_empty()
    });
}

fn checkbox(app: &App, label: &str) -> Option<(bool, bool)> {
    inspect::find(app.slint(), AccessibleRole::Checkbox, label)
        .map(|item| (tools::checked(&item), tools::enabled(&item)))
}

/// Tab through the open dialog of one file from its destination field: Links, the
/// preserve option, the queueing option, Cancel, the primary action (`verb`), then around
/// to the destination field again (past the header's close button when it takes focus).
fn tab_order(app: &mut App, verb: &str) -> Result<(), String> {
    // The preserve option is a stop wherever it is offered (every Copy and Move).
    let offered = checkbox(app, PRESERVE).is_some_and(|(_, enabled)| enabled);
    let stops: Vec<(AccessibleRole, &str)> = [
        (AccessibleRole::Combobox, support::LINKS_LABEL),
        (AccessibleRole::Checkbox, PRESERVE),
        (AccessibleRole::Checkbox, support::WAIT_OPTION),
        (AccessibleRole::Button, "Cancel"),
        (AccessibleRole::Button, verb),
    ]
    .into_iter()
    .filter(|(_, label)| offered || *label != PRESERVE)
    .collect();
    // The dialog opens with the destination field focused.
    if !support::focus_inside(app.slint(), AccessibleRole::TextInput, "Destination folder") {
        return Err(format!(
            "the {verb} dialog does not open with its destination field focused"
        ));
    }
    for (role, label) in &stops {
        input::key(app.slint(), Key::Tab);
        app.settle();
        if !support::focus_inside(app.slint(), *role, label) {
            return Err(format!(
                "Tab in the {verb} dialog did not reach {label:?} next (stops {:?})",
                stops.iter().map(|(_, label)| *label).collect::<Vec<_>>()
            ));
        }
    }
    // Around: the header's close button may take one stop before the field.
    for _ in 0..2 {
        input::key(app.slint(), Key::Tab);
        app.settle();
        if support::focus_inside(app.slint(), AccessibleRole::TextInput, "Destination folder") {
            break;
        }
    }
    if !support::focus_inside(app.slint(), AccessibleRole::TextInput, "Destination folder") {
        return Err(format!(
            "Tab after {verb} did not come back to the destination field"
        ));
    }
    // Shift+Tab goes back to the primary action (or the close button first).
    input::chord(app.slint(), &[Key::Shift], Key::Tab);
    app.settle();
    if !support::focus_inside(app.slint(), AccessibleRole::Button, verb) {
        input::chord(app.slint(), &[Key::Shift], Key::Tab);
        app.settle();
    }
    if !support::focus_inside(app.slint(), AccessibleRole::Button, verb) {
        return Err(format!(
            "Shift+Tab from the destination field did not go back to {verb}"
        ));
    }
    Ok(())
}

/// The open dialog's Links choice on a fixed local folder, once the folder's link facts
/// arrived (after the trace `before`): "Keep links as links" of exactly its two entries,
/// without a reason or a warning (shown or read), and without a visible caption (both
/// entries name links), yet named "Links" for a screen reader.
fn check_links(app: &mut App, before: &[String], what: &str, failures: &mut Vec<String>) {
    if !support::wait_link_facts(app, before) {
        failures.push(format!(
            "{what}: the destination's link facts did not arrive within 10 s"
        ));
        return;
    }
    let Some(view) = support::link_view(app) else {
        failures.push(format!("{what} offers no Links choice"));
        return;
    };
    let mut differences = view.plain_default();
    if inspect::find(app.slint(), AccessibleRole::Combobox, support::LINKS_LABEL).is_none() {
        differences.push(format!(
            "no box is named {:?} for a screen reader",
            support::LINKS_LABEL
        ));
    }
    if inspect::visible_texts(app.slint())
        .iter()
        .any(|text| text == support::LINKS_LABEL)
    {
        differences.push(format!(
            "it still shows the caption {:?}, which its entries repeat",
            support::LINKS_LABEL
        ));
    }
    if !differences.is_empty() {
        failures.push(format!(
            "{what}'s Links choice on a fixed local folder: {}",
            differences.join("; ")
        ));
    }
}

/// Type `destination` and press Enter: the dialog must refuse it and stay open.
fn refused(app: &mut App, destination: &str, start: usize) -> Result<(), String> {
    input::fill(app.slint(), "Destination folder", destination)?;
    app.settle();
    input::key(app.slint(), Key::Return);
    // An empty field confirms nothing; anything else is refused with its reason (a typed
    // folder once the backend checked it), or admitted as a job, which closes the dialog.
    if !destination.is_empty() {
        app.until(
            "the dialog to refuse or admit the destination",
            move |app| {
                !app.window.get_copy_draft_error().is_empty()
                    || app.window.get_copy_draft_token().is_empty()
                    || app.copies.len() > start
            },
        );
    }
    app.settle();
    let open = !app.window.get_copy_draft_token().is_empty();
    let error = app.window.get_copy_draft_error().to_string();
    if !open || app.copies.len() != start {
        let jobs: std::collections::BTreeSet<_> =
            app.copies[start..].iter().map(|job| job.id).collect();
        return Err(format!(
            "the destination {destination:?} was accepted (dialog open {open}, jobs started {})",
            jobs.len()
        ));
    }
    if !destination.is_empty() && error != MISSING {
        return Err(format!(
            "the destination {destination:?} shows {error:?} instead of {MISSING:?}"
        ));
    }
    // Enter with an empty field says, under the field, what is missing.
    if destination.is_empty()
        && !inspect::visible_texts(app.slint())
            .iter()
            .any(|text| text == TYPE_FIRST)
    {
        return Err(format!(
            "Enter with an empty destination does not say {TYPE_FIRST:?}: {:?}",
            inspect::visible_texts(app.slint())
        ));
    }
    Ok(())
}

pub fn copy_options(context: &mut Context) -> Result<(), String> {
    let source = support::scratch(context, "source")?;
    let target = support::scratch(context, "target")?;
    let quick = support::scratch(context, "quick")?;
    let full = support::scratch(context, "full")?;
    let many = support::scratch(context, "many-target")?;
    fs::write(source.join("a.txt"), "a\n").map_err(|error| io_error(&source, error))?;
    fs::create_dir(source.join("dir")).map_err(|error| io_error(&source, error))?;
    fs::write(source.join("dir/x.txt"), "x\n").map_err(|error| io_error(&source, error))?;
    let huge = context.fixture.path("sizes/huge.bin");
    if fs::hard_link(&huge, source.join("huge.bin")).is_err() {
        fs::copy(&huge, source.join("huge.bin")).map_err(|error| io_error(&huge, error))?;
    }
    copy_tree(&context.fixture.path("many"), &source.join("many"))?;
    support::stage(context, &["deep", "docs"], &source)?;
    let tree_target = support::scratch(context, "tree-target")?;
    let size = fs::metadata(&huge)
        .map_err(|error| io_error(&huge, error))?
        .len();
    let store = SettingsStore::new();
    let mut config = Config::new(&source, &target);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let mut failures = Vec::new();

    // F5 on a file: the dialog's summary, destination, options and policy.
    panel::select_row(&mut app, LEFT, "a.txt")?;
    let trace = app.recent_events();
    open_dialog(&mut app, Key::F5);
    let body = app.window.get_copy_draft_body().to_string();
    let source_shown = app.window.get_copy_draft_source().to_string();
    let policy = app.window.get_copy_draft_policy().to_string();
    let destination =
        support::accessible_value(app.slint(), AccessibleRole::TextInput, "Destination folder");
    if !body.contains("a.txt") || !source_shown.contains(&source.display().to_string()) {
        failures.push(format!(
            "the Copy dialog's summary is {body:?}, its source {source_shown:?}"
        ));
    }
    if destination.as_deref() != Some(&*target.to_string_lossy()) {
        failures.push(format!(
            "the destination is {destination:?}, not the other panel's folder"
        ));
    }
    if !policy.contains(FULL) {
        failures.push(format!("the Copy dialog's policy line is {policy:?}"));
    }
    let preserve = checkbox(&app, PRESERVE);
    if preserve != Some((false, true)) {
        failures.push(format!(
            "{PRESERVE:?} for one file is {preserve:?} (checked, enabled)"
        ));
    }
    let wait = checkbox(&app, support::WAIT_OPTION);
    if wait != Some((false, true)) {
        failures.push(format!("{:?} is {wait:?}", support::WAIT_OPTION));
    }
    // The Links choice replaced the former link check boxes: none of them is left.
    for former in ["Follow links", "Follow directory links"] {
        if checkbox(&app, former).is_some() {
            failures.push(format!("the Copy dialog still offers {former:?}"));
        }
    }
    check_links(&mut app, &trace, "Copy", &mut failures);
    app.screenshot(context, "copy-dialog", "F5 of one file");

    // Destinations the dialog refuses: a relative path, nothing, then a missing folder.
    let start = app.copies.len();
    for destination in ["relative/dir".to_owned(), String::new()] {
        if let Err(error) = refused(&mut app, &destination, start) {
            failures.push(error);
            break;
        }
    }
    let missing = target.join("missing").to_string_lossy().into_owned();
    match refused(&mut app, &missing, start) {
        Ok(()) => {
            app.screenshot(
                context,
                "copy-dialog-refused",
                "A destination that does not exist",
            );
            close_dialog(&mut app);
        }
        Err(error) if app.copies.len() > start => {
            // Admitted as a job, which then asks in its own window: answer Cancel job,
            // so nothing is copied and no question is left for later steps.
            let window = support::wait_decision(&mut app, "the admitted job's question");
            let question = support::describe_window(&window);
            app.screenshot_window(
                context,
                &window,
                "copy-missing-destination",
                "A missing destination folder admitted as a job",
            );
            support::answer(&mut app, &window, &support::Choice::CancelJob)?;
            let progress = support::wait_job(&mut app, start);
            failures.push(format!(
                "{error}: the job asked {question} and ended {:?} after Cancel job \
                 (finding ACC-OPT-01)",
                progress.state
            ));
            support::dismiss_all(&mut app);
        }
        Err(error) => {
            failures.push(error);
            close_dialog(&mut app);
        }
    }
    if !support::names_on_disk(&target).is_empty() {
        failures.push(format!(
            "Escape after refused destinations left {:?}",
            support::names_on_disk(&target)
        ));
    }

    // A folder keeps its times and attributes too; Move offers the same Links choice and
    // renames.
    panel::select_row(&mut app, LEFT, "dir")?;
    open_dialog(&mut app, Key::F5);
    let preserve = checkbox(&app, PRESERVE);
    if preserve != Some((false, true)) {
        failures.push(format!(
            "{PRESERVE:?} for a folder is {preserve:?} (checked, enabled)"
        ));
    }
    close_dialog(&mut app);
    panel::select_row(&mut app, LEFT, "a.txt")?;
    let trace = app.recent_events();
    open_dialog(&mut app, Key::F6);
    let policy = app.window.get_copy_draft_policy().to_string();
    if !policy.contains(MOVE_POLICY) {
        failures.push(format!("the Move dialog's policy line is {policy:?}"));
    }
    for former in ["Follow links", "Follow directory links"] {
        if checkbox(&app, former).is_some() {
            failures.push(format!("the Move dialog still offers {former:?}"));
        }
    }
    check_links(&mut app, &trace, "Move", &mut failures);
    app.screenshot(context, "move-dialog", "F6 of one file");
    // Tab visits the destination, the exact-time option, Links, the queueing option,
    // Cancel and Move in the order shown, and wraps.
    if let Err(error) = tab_order(&mut app, "Move") {
        failures.push(error);
    }
    // Move refuses the same destinations as Copy, and its Cancel button moves nothing.
    let start = app.copies.len();
    for destination in [
        "relative/dir".to_owned(),
        String::new(),
        target.join("missing").to_string_lossy().into_owned(),
    ] {
        if let Err(error) = refused(&mut app, &destination, start) {
            failures.push(format!("Move: {error}"));
            break;
        }
    }
    match input::click_button(app.slint(), "Cancel") {
        Err(error) => failures.push(format!("the Move dialog's Cancel: {error}")),
        Ok(()) => app.until("Cancel to close the Move dialog", |app| {
            app.window.get_copy_draft_token().is_empty()
        }),
    }
    if app.copies.len() != start || !source.join("a.txt").is_file() {
        failures.push("Cancel in the Move dialog moved something".into());
    }

    // Full verification (the default) reads every copied byte back.
    panel::select_row(&mut app, LEFT, "huge.bin")?;
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(full.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    if let Err(error) = support::completed(&run, "F5 huge.bin with full verification") {
        failures.push(error);
    }
    if run.progress.verification != TransferVerification::FullSha256
        || run.progress.verified_bytes != size
    {
        failures.push(format!(
            "full verification read {} of {size} bytes back ({:?})",
            run.progress.verified_bytes, run.progress.verification
        ));
    }

    // Options › General: size and samples instead.
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    options::set_checked(&mut app, VERIFY_OPTION, false)?;
    if !options::ok(&mut app, &store)? {
        failures.push("OK did not save the verification choice".into());
    }
    panel::select_row(&mut app, LEFT, "huge.bin")?;
    open_dialog(&mut app, Key::F5);
    let policy = app.window.get_copy_draft_policy().to_string();
    close_dialog(&mut app);
    if !policy.contains(QUICK) {
        failures.push(format!(
            "with quick verification the policy line is {policy:?}"
        ));
    }
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(quick.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    if let Err(error) = support::completed(&run, "F5 huge.bin with quick verification") {
        failures.push(error);
    }
    // Three samples of 64 KiB; a file up to 192 KiB is compared whole.
    if run.progress.verification != TransferVerification::SizeAndSamples
        || (size > 3 * 65_536 && run.progress.verified_bytes > 3 * 65_536)
    {
        failures.push(format!(
            "quick verification read {} of {size} bytes back ({:?})",
            run.progress.verified_bytes, run.progress.verification
        ));
    }
    for folder in [&full, &quick] {
        if fs::read(folder.join("huge.bin")).ok() != fs::read(&huge).ok() {
            failures.push(format!(
                "{} holds a wrong copy",
                folder.join("huge.bin").display()
            ));
        }
    }
    context.note(format!(
        "huge.bin ({size} bytes) read back {} bytes with full verification and {} with size and samples",
        size, run.progress.verified_bytes
    ));

    // Many small files in one job (each written, synced and read back: minutes on a
    // busy disk).
    panel::select_row(&mut app, LEFT, "many")?;
    let started = Instant::now();
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(many.as_path()),
            limit: Some(std::time::Duration::from_secs(300)),
            ..TransferSpec::copy()
        },
    )?;
    if let Err(error) = support::completed(&run, "F5 of many/") {
        failures.push(error);
    }
    let differences = compare_trees(
        &tree_facts(&source.join("many"))?,
        &tree_facts(&many.join("many"))?,
        false,
    );
    if !differences.is_empty() {
        failures.push(format!(
            "many/ copied with differences: {:?}",
            differences.iter().take(5).collect::<Vec<_>>()
        ));
    }
    context.note(format!(
        "many/ ({} files) copied in {:.1} s: {}",
        run.progress.files_completed,
        started.elapsed().as_secs_f64(),
        support::describe_run(&run)
    ));

    // Rows marked at different depths of two inline tree branches are copied together,
    // each as its own folder.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    support::toggle_tree(&mut app, "docs")?;
    support::wait_tree_depth(&mut app, "page_files", Some(1));
    support::toggle_tree(&mut app, "deep")?;
    support::wait_tree_depth(&mut app, "level-01", Some(1));
    support::toggle_tree(&mut app, "level-01")?;
    support::wait_tree_depth(&mut app, "level-02", Some(2));
    for name in ["page_files", "level-02"] {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    app.screenshot(
        context,
        "tree-multi-level",
        "Rows marked at depth 1 and 2 of the inline tree",
    );
    let run = support::transfer(
        &mut app,
        context,
        TransferSpec {
            destination: Some(tree_target.as_path()),
            ..TransferSpec::copy()
        },
    )?;
    if let Err(error) = support::completed(&run, "F5 of rows marked at two tree depths") {
        failures.push(error);
    }
    for (copied, original) in [
        ("page_files", "docs/page_files"),
        ("level-02", "deep/level-01/level-02"),
    ] {
        let differences = compare_trees(
            &tree_facts(&source.join(original))?,
            &tree_facts(&tree_target.join(copied)).unwrap_or_default(),
            false,
        );
        if differences.is_empty() && tree_target.join(copied).is_dir() {
            continue;
        }
        failures.push(format!(
            "the tree row {original} was copied with differences: {:?}",
            differences.iter().take(5).collect::<Vec<_>>()
        ));
    }
    if support::names_on_disk(&tree_target) != ["level-02", "page_files"] {
        failures.push(format!(
            "the tree selection's target holds {:?}",
            support::names_on_disk(&tree_target)
        ));
    }
    app.shutdown()?;
    support::report(&failures)
}
