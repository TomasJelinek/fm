//! Filename search and the panel filter in depth, and their combinations with marks,
//! sorting, hidden files and the other panel. Search: matches follow Up/Down (no wrap),
//! Home/End and Page Up/Down; Backspace removes one character and an empty query ends
//! the search; Space extends the query; Insert ends it and marks the match; Unicode
//! names match without case; a query survives sorting and Ctrl+R and ends when the
//! folder changes. Filter: Regex from the drive bar's funnel, Case sensitive, the
//! indicator above the list (opens the filter; its × turns it off and keeps the
//! expression), the history menu, Cancel keeping the applied filter, an invalid regular
//! expression refused with the listing kept; Ctrl+A, Ctrl+Shift+A, sorting and search
//! inside a filtered list; hidden files with a filter and Ctrl+A; each panel's own filter.
//! Commands never wait for the search (aac18026): F5, F8, Ctrl+F8 and F4 close it and act
//! on the marks or the focused shown row, also when pressed before its matches arrive;
//! Enter on a name that matches nothing opens nothing and says so. The applied filter
//! and the filter history come back after a restart.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect, panel,
    scenarios::panels_support::{self as ps, LEFT, RIGHT},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter},
};
use std::{path::Path, time::Duration};

const UNICODE: &str = "ŽLUŤOUČKÝ kůň.txt";

fn search(app: &App) -> (bool, String) {
    (
        app.window.get_left_search_active(),
        app.window.get_left_search_query().to_string(),
    )
}

/// The shown names (no `..`) starting with `prefix`, ignoring case, in displayed order.
fn matches(app: &App, prefix: &str) -> Vec<String> {
    let prefix = prefix.to_lowercase();
    ps::shown(app, LEFT)
        .into_iter()
        .filter(|name| name.to_lowercase().starts_with(&prefix))
        .collect()
}

fn press(app: &mut App, key: Key) {
    input::key(app.slint(), key);
    app.settle();
}

fn expect_cursor(app: &App, what: &str, wanted: Option<&str>, failures: &mut Vec<String>) {
    let cursor = ps::cursor(app, LEFT);
    if cursor.as_deref() != wanted {
        failures.push(format!(
            "{what}: the cursor is on {cursor:?}, expected {wanted:?}"
        ));
    }
}

fn filter_open(app: &App) -> bool {
    app.window.get_panel_filter_view().open
}

fn filter_label(app: &App, pane: PaneId) -> String {
    match pane {
        PaneId::Left => app.window.get_left_filter_label(),
        PaneId::Right => app.window.get_right_filter_label(),
    }
    .to_string()
}

/// Wait until the left panel's top-level names, sorted, are `wanted`.
fn lists(app: &mut App, pane: PaneId, wanted: &[&str]) -> bool {
    let wanted = ps::sorted(wanted);
    ps::wait(app, Duration::from_secs(10), |app| {
        let mut names = ps::top_level(app, pane);
        names.sort();
        names == wanted
    })
}

fn open_filter(app: &mut App, pane: PaneId) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    if !ps::wait(app, Duration::from_secs(5), filter_open) {
        return Err("Ctrl+F12 opened no Panel filter".into());
    }
    Ok(())
}

fn ok(app: &mut App) -> Result<(), String> {
    input::click_button(app.slint(), "OK")?;
    if !ps::wait(app, Duration::from_secs(10), |app| !filter_open(app)) {
        return Err(format!(
            "OK left the Panel filter open (error {:?})",
            app.window.get_panel_filter_view().error
        ));
    }
    app.idle(Duration::from_millis(100));
    Ok(())
}

pub fn search_filter(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "search")?;
    ps::folder(&folder.join("alpine"))?;
    ps::write(&folder.join("alpine/inner.txt"), "inner")?;
    for (name, size) in [
        ("alpha.txt", 10),
        ("Alpha2.log", 30),
        ("beta.txt", 20),
        (UNICODE, 5),
        ("zeta.dat", 40),
        (".hidden-a.txt", 50),
    ] {
        ps::write(&folder.join(name), vec![b'x'; size])?;
    }
    // Windows hides by the hidden attribute, not by name.
    crate::fixtures::hide(&folder.join(".hidden-a.txt"))?;
    let store = crate::options_support::SettingsStore::new();
    let mut config = Config::new(&folder, &folder);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let mut failures = Vec::new();
    searching(&mut app, &folder, &mut failures)?;
    filtering(&mut app, context, &mut failures)?;
    combinations(&mut app, &mut failures)?;
    // The applied filter and the filter history are saved: a restart shows both.
    open_filter(&mut app, LEFT)?;
    crate::options_support::choose(&mut app, "Filter mode", "Regex")?;
    input::fill(app.slint(), "File filter expression", "^al")?;
    if ps::checked(&app, "Case sensitive") == Some(true) {
        input::click(app.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
        app.settle();
    }
    ok(&mut app)?;
    if !lists(&mut app, LEFT, &["alpine", "alpha.txt", "Alpha2.log"]) {
        failures.push(format!(
            "Regex ^al before the restart lists {:?}",
            ps::top_level(&app, LEFT)
        ));
    }
    app.shutdown()?;
    drop(app);
    restarted_filter(&store, &folder, &mut failures)?;
    commands(context, &mut failures)?;
    support::report(&failures)
}

/// After a restart on the saved settings the left panel applies its Regex ^al filter
/// again (indicator and listing) and the filter history still lists the filters applied
/// before, case-sensitive one included.
fn restarted_filter(
    store: &std::sync::Arc<crate::options_support::SettingsStore>,
    folder: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    if filter_label(&app, LEFT) != "Filter: regex ^al"
        || !lists(&mut app, LEFT, &["alpine", "alpha.txt", "Alpha2.log"])
    {
        failures.push(format!(
            "after a restart the left filter is {:?} listing {:?}, expected Regex ^al",
            filter_label(&app, LEFT),
            ps::top_level(&app, LEFT)
        ));
    }
    if !filter_label(&app, RIGHT).is_empty() {
        failures.push(format!(
            "after a restart the right panel shows the filter {:?}",
            filter_label(&app, RIGHT)
        ));
    }
    open_filter(&mut app, LEFT)?;
    input::click(app.slint(), AccessibleRole::Combobox, "Filter history")?;
    app.settle();
    let labels: Vec<String> = ps::popup_buttons(&app)
        .into_iter()
        .map(|(label, _)| label)
        .filter(|label| label.contains(": "))
        .collect();
    for wanted in ["Regex: ^al", "Regex (case sensitive): ^al"] {
        if !labels.iter().any(|label| label == wanted) {
            failures.push(format!(
                "after a restart the filter history lacks {wanted:?}: {labels:?}"
            ));
        }
    }
    support::dismiss_all(&mut app);
    app.shutdown()
}

fn searching(app: &mut App, folder: &Path, failures: &mut Vec<String>) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    press(app, Key::Home);
    type_search(app, "al");
    let found = matches(app, "al");
    if found.len() != 3 || search(app) != (true, "al".into()) {
        failures.push(format!(
            "typing al: search {:?}, matches {found:?} (expected three)",
            search(app)
        ));
    }
    expect_cursor(
        app,
        "typing al",
        found.first().map(String::as_str),
        failures,
    );
    // Down and Up visit matches without wrapping; Home/End and Page Up/Down follow them.
    let last = found.last().map(String::as_str);
    for (key, wanted) in [
        (Key::DownArrow, found.get(1).map(String::as_str)),
        (Key::DownArrow, found.get(2).map(String::as_str)),
        (Key::DownArrow, found.get(2).map(String::as_str)),
        (Key::UpArrow, found.get(1).map(String::as_str)),
        (Key::Home, found.first().map(String::as_str)),
        (Key::UpArrow, found.first().map(String::as_str)),
        (Key::End, last),
        (Key::PageUp, found.first().map(String::as_str)),
        (Key::PageDown, last),
    ] {
        press(app, key);
        expect_cursor(
            app,
            &format!("{key:?} while searching al"),
            wanted,
            failures,
        );
    }
    if search(app) != (true, "al".into()) {
        failures.push(format!(
            "moving among matches changed the search: {:?}",
            search(app)
        ));
    }
    // Backspace removes one character; an empty query ends the search.
    press(app, Key::Backspace);
    if search(app) != (true, "a".into()) {
        failures.push(format!("Backspace in \"al\" left {:?}", search(app)));
    }
    press(app, Key::Backspace);
    if search(app).0 {
        failures.push(format!(
            "Backspace of the last character left {:?}",
            search(app)
        ));
    }
    // Space extends the query; Insert ends the search and marks the match.
    type_search(app, "be");
    expect_cursor(app, "typing be", Some("beta.txt"), failures);
    press(app, Key::Space);
    if search(app) != (true, "be ".into()) {
        failures.push(format!("Space while searching gave {:?}", search(app)));
    }
    press(app, Key::Backspace);
    expect_cursor(app, "Backspace back to be", Some("beta.txt"), failures);
    press(app, Key::Insert);
    if search(app).0 || ps::marked(app, LEFT) != ["beta.txt"] {
        failures.push(format!(
            "Insert while searching be: search {:?}, marks {:?}, expected the search ended and beta.txt marked",
            search(app),
            ps::marked(app, LEFT)
        ));
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    // Unicode names match without case.
    type_search(app, "žl");
    expect_cursor(app, "typing žl", Some(UNICODE), failures);
    press(app, Key::Escape);
    if search(app).0 {
        failures.push("Escape did not end the search".into());
    }
    // A query survives sorting and Ctrl+R.
    type_search(app, "al");
    input::chord(app.slint(), &[Key::Control], Key::F6);
    app.settle();
    if search(app) != (true, "al".into())
        || !matches(app, "al").contains(&ps::cursor(app, LEFT).unwrap_or_default())
    {
        failures.push(format!(
            "after Ctrl+F6 while searching: search {:?}, cursor {:?}",
            search(app),
            ps::cursor(app, LEFT)
        ));
    }
    let listings = app.listings[0];
    input::chord(app.slint(), &[Key::Control], "r");
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.listings[0] > listings && !app.loading(LEFT)
    }) {
        failures.push("Ctrl+R while searching did not list the folder again".into());
    }
    if search(app) != (true, "al".into()) {
        failures.push(format!("Ctrl+R ended the search: {:?}", search(app)));
    }
    // Back to name order, then Enter on the folder match: the search ends there.
    for _ in 0..2 {
        if app.window.get_left_sort() == 0 && !app.window.get_left_descending() {
            break;
        }
        input::chord(app.slint(), &[Key::Control], Key::F3);
        app.settle();
    }
    press(app, Key::Home);
    expect_cursor(app, "Home while searching al", Some("alpine"), failures);
    press(app, Key::Return);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.location(LEFT).as_deref() == Some(folder.join("alpine").as_path())
    }) {
        failures.push(format!(
            "Enter on the alpine match went to {:?}",
            app.location(LEFT)
        ));
    } else if search(app).0 || !search(app).1.is_empty() {
        failures.push(format!(
            "opening a folder kept the search {:?}",
            search(app)
        ));
    }
    press(app, Key::Backspace);
    panel::wait_location(app, LEFT, folder);
    Ok(())
}

fn filtering(
    app: &mut App,
    context: &mut Context,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let all = [
        "alpine",
        "alpha.txt",
        "Alpha2.log",
        "beta.txt",
        UNICODE,
        "zeta.dat",
        ".hidden-a.txt",
    ];
    // The drive bar's funnel opens the filter; Regex ^al (letter case ignored).
    panel::focus_list(app, LEFT)?;
    ps::click_starting(app, "Left panel filter")?;
    if !ps::wait(app, Duration::from_secs(5), filter_open) {
        failures.push("the funnel opened no Panel filter".into());
        return Ok(());
    }
    crate::options_support::choose(app, "Filter mode", "Regex")?;
    input::fill(app.slint(), "File filter expression", "^al")?;
    ok(app)?;
    if !lists(app, LEFT, &["alpine", "alpha.txt", "Alpha2.log"]) {
        failures.push(format!("Regex ^al lists {:?}", ps::top_level(app, LEFT)));
    }
    if filter_label(app, LEFT) != "Filter: regex ^al" {
        failures.push(format!(
            "the filter indicator says {:?}",
            filter_label(app, LEFT)
        ));
    }
    app.screenshot(context, "regex-filter", "Regex ^al with its indicator");
    // The indicator opens the filter with its draft; Case sensitive narrows it.
    match inspect::find(app.slint(), AccessibleRole::Button, "Filter: regex ^al") {
        None => failures.push("no filter indicator button above the list".into()),
        Some(item) => {
            let center = inspect::center(&item);
            input::click_at(app.slint(), center);
            app.settle();
            if !ps::wait(app, Duration::from_secs(5), filter_open) {
                failures.push("clicking the filter indicator opened nothing".into());
            } else {
                let view = app.window.get_panel_filter_view();
                if view.expression != "^al" || view.mode != 1 {
                    failures.push(format!(
                        "the reopened filter shows mode {} and {:?}",
                        view.mode, view.expression
                    ));
                }
                input::click(app.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
                app.settle();
                ok(app)?;
                if !lists(app, LEFT, &["alpine", "alpha.txt"]) {
                    failures.push(format!(
                        "case-sensitive Regex ^al lists {:?}",
                        ps::top_level(app, LEFT)
                    ));
                }
            }
        }
    }
    // The indicator's × turns the filter off and keeps the expression.
    match inspect::find(app.slint(), AccessibleRole::Button, "Clear panel filter") {
        None => failures.push("the filter indicator has no × (Clear panel filter)".into()),
        Some(item) => {
            input::click_at(app.slint(), inspect::center(&item));
            app.settle();
            if !lists(app, LEFT, &all) || !filter_label(app, LEFT).is_empty() {
                failures.push(format!(
                    "the indicator's ×: label {:?}, listing {:?}",
                    filter_label(app, LEFT),
                    ps::top_level(app, LEFT)
                ));
            }
            if app.window.get_focused_pane() != 0 {
                failures.push("after the indicator's × the file list has no keyboard focus".into());
            }
            open_filter(app, LEFT)?;
            let view = app.window.get_panel_filter_view();
            if view.expression != "^al" || view.mode != 1 || !view.case_sensitive {
                failures.push(format!(
                    "after × the filter forgot its draft: mode {}, {:?}, case {}",
                    view.mode, view.expression, view.case_sensitive
                ));
            }
            input::click_button(app.slint(), "Cancel")?;
            ps::wait(app, Duration::from_secs(5), |app| !filter_open(app));
        }
    }
    // The history lists both applied filters; choosing one fills the draft.
    open_filter(app, LEFT)?;
    input::click(app.slint(), AccessibleRole::Combobox, "Filter history")?;
    app.settle();
    let labels: Vec<String> = ps::popup_buttons(app)
        .into_iter()
        .map(|(label, _)| label)
        .filter(|label| label.starts_with("Regex"))
        .collect();
    if !labels.contains(&"Regex: ^al".to_owned())
        || !labels.contains(&"Regex (case sensitive): ^al".to_owned())
    {
        failures.push(format!("the filter history lists {labels:?}"));
    }
    match ps::click_popup_button(app, "Regex: ^al") {
        Err(error) => failures.push(format!("filter history: {error}")),
        Ok(()) => {
            let field = ps::value(app, AccessibleRole::TextInput, "File filter expression");
            if ps::checked(app, "Case sensitive") != Some(false) || field.as_deref() != Some("^al")
            {
                failures.push(
                    "choosing Regex: ^al in the history did not fill its draft (case off)".into(),
                );
            }
            ok(app)?;
            if !lists(app, LEFT, &["alpine", "alpha.txt", "Alpha2.log"]) {
                failures.push(format!(
                    "the history's Regex ^al lists {:?}",
                    ps::top_level(app, LEFT)
                ));
            }
        }
    }
    // Help and F1 show the syntax over the filter; Escape closes only the help.
    open_filter(app, LEFT)?;
    let help = |app: &App| {
        !app.window
            .global::<midnight_ui::PanelFilterPresentation>()
            .get_help_token()
            .is_empty()
    };
    for how in ["the Help button", "F1"] {
        if how == "F1" {
            input::key(app.slint(), Key::F1);
        } else {
            input::click_button(app.slint(), "Help")?;
        }
        if !ps::wait(app, Duration::from_secs(5), help) {
            failures.push(format!("{how} in the Panel filter showed no help"));
            continue;
        }
        let texts = ps::dialog_texts(app);
        if !texts.iter().any(|text| text.text.contains('*')) {
            failures.push(format!(
                "the filter help ({how}) shows no mask syntax: {:?}",
                texts
                    .iter()
                    .map(|text| &text.text)
                    .take(8)
                    .collect::<Vec<_>>()
            ));
        }
        input::key(app.slint(), Key::Escape);
        if !ps::wait(app, Duration::from_secs(5), |app| !help(app)) {
            failures.push(format!("Escape did not close the filter help ({how})"));
        } else if !filter_open(app) {
            failures.push(format!(
                "Escape in the filter help ({how}) also closed the filter"
            ));
            open_filter(app, LEFT)?;
        }
    }
    input::click_button(app.slint(), "Cancel")?;
    ps::wait(app, Duration::from_secs(5), |app| !filter_open(app));
    // Cancel keeps the applied filter.
    open_filter(app, LEFT)?;
    input::fill(app.slint(), "File filter expression", "zzz")?;
    input::click_button(app.slint(), "Cancel")?;
    ps::wait(app, Duration::from_secs(5), |app| !filter_open(app));
    if filter_label(app, LEFT) != "Filter: regex ^al"
        || !lists(app, LEFT, &["alpine", "alpha.txt", "Alpha2.log"])
    {
        failures.push(format!(
            "Cancel changed the filter: {:?}, {:?}",
            filter_label(app, LEFT),
            ps::top_level(app, LEFT)
        ));
    }
    // An invalid regular expression is refused in the dialog; the listing stays.
    open_filter(app, LEFT)?;
    input::fill(app.slint(), "File filter expression", "(")?;
    input::click_button(app.slint(), "OK")?;
    let refused = ps::wait(app, Duration::from_secs(10), |app| {
        let view = app.window.get_panel_filter_view();
        view.open && !view.busy && !view.error.is_empty()
    });
    if !refused {
        failures.push(format!(
            "an invalid regular expression was not refused in the dialog (open {}, error {:?})",
            filter_open(app),
            app.window.get_panel_filter_view().error
        ));
    } else {
        app.screenshot(
            context,
            "invalid-regex",
            "An invalid regular expression refused",
        );
    }
    support::dismiss_all(app);
    if filter_label(app, LEFT) != "Filter: regex ^al"
        || !lists(app, LEFT, &["alpine", "alpha.txt", "Alpha2.log"])
    {
        failures.push(format!(
            "the invalid expression changed the filter: {:?}, {:?}",
            filter_label(app, LEFT),
            ps::top_level(app, LEFT)
        ));
    }
    Ok(())
}

fn combinations(app: &mut App, failures: &mut Vec<String>) -> Result<(), String> {
    // Inside the filtered list (Regex ^al): Ctrl+A, Ctrl+Shift+A, sorting, search.
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "a");
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT) == ps::sorted(&["alpha.txt", "Alpha2.log"])
    }) {
        failures.push(format!(
            "Ctrl+A in the filtered list marked {:?}",
            ps::marked(app, LEFT)
        ));
    }
    input::chord(app.slint(), &[Key::Control, Key::Shift], "a");
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT) == ps::sorted(&["alpine", "alpha.txt", "Alpha2.log"])
    }) {
        failures.push(format!(
            "Ctrl+Shift+A in the filtered list marked {:?}",
            ps::marked(app, LEFT)
        ));
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    input::chord(app.slint(), &[Key::Control], Key::F6);
    app.settle();
    let wanted_order = vec!["alpha.txt".to_owned(), "Alpha2.log".to_owned()];
    let descending = app.window.get_left_descending();
    let mut expected = wanted_order.clone();
    if descending {
        expected.reverse();
    }
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::top_level(app, LEFT)
            .into_iter()
            .filter(|name| name != "alpine")
            .collect::<Vec<_>>()
            == expected
    }) {
        failures.push(format!(
            "sorting the filtered list by size gives {:?}, expected {expected:?}",
            ps::top_level(app, LEFT)
        ));
    }
    type_search(app, "alpha2");
    expect_cursor(
        app,
        "searching alpha2 in the filtered list",
        Some("Alpha2.log"),
        failures,
    );
    press(app, Key::Escape);
    // The right panel's own filter; the left one stays.
    open_filter(app, RIGHT)?;
    crate::options_support::choose(app, "Filter mode", "Masks")?;
    input::fill(app.slint(), "File filter expression", "*.dat")?;
    ok(app)?;
    if !lists(app, RIGHT, &["alpine", "zeta.dat"]) || filter_label(app, LEFT) != "Filter: regex ^al"
    {
        failures.push(format!(
            "the right panel's *.dat lists {:?}; the left filter is {:?}",
            ps::top_level(app, RIGHT),
            filter_label(app, LEFT)
        ));
    }
    open_filter(app, RIGHT)?;
    input::click_button(app.slint(), "Show all")?;
    ps::wait(app, Duration::from_secs(5), |app| !filter_open(app));
    // Hidden files hidden plus a *.txt filter: Ctrl+A marks only the shown .txt files,
    // and showing hidden files again leaves the dot-file unmarked.
    open_filter(app, LEFT)?;
    crate::options_support::choose(app, "Filter mode", "Masks")?;
    input::fill(app.slint(), "File filter expression", "*.txt")?;
    ok(app)?;
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "h");
    if !lists(app, LEFT, &["alpine", "alpha.txt", "beta.txt", UNICODE]) {
        failures.push(format!(
            "*.txt with hidden files hidden lists {:?}",
            ps::top_level(app, LEFT)
        ));
    }
    input::chord(app.slint(), &[Key::Control], "a");
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT) == ps::sorted(&["alpha.txt", "beta.txt", UNICODE])
    }) {
        failures.push(format!(
            "Ctrl+A with *.txt and hidden files hidden marked {:?}",
            ps::marked(app, LEFT)
        ));
    }
    input::chord(app.slint(), &[Key::Control], "h");
    if !lists(
        app,
        LEFT,
        &["alpine", "alpha.txt", "beta.txt", UNICODE, ".hidden-a.txt"],
    ) {
        failures.push(format!(
            "*.txt with hidden files shown lists {:?}",
            ps::top_level(app, LEFT)
        ));
    }
    if ps::marked(app, LEFT).contains(&".hidden-a.txt".to_owned()) {
        failures.push("showing hidden files again marked the dot-file".into());
    }
    input::chord(app.slint(), &[Key::Control], "d");
    open_filter(app, LEFT)?;
    input::click_button(app.slint(), "Show all")?;
    ps::wait(app, Duration::from_secs(5), |app| !filter_open(app));
    Ok(())
}

/// Type `text` into the focused list and wait until its quick search has its matches
/// (the search is computed in the background; a check right after one settle races it).
fn type_search(app: &mut App, text: &str) {
    input::type_text(app.slint(), text);
    app.settle();
    ps::wait(app, Duration::from_secs(5), |app| {
        !app.window.get_left_search_pending()
    });
}

/// The left quick search is closed: not active and its text empty.
fn search_closed(app: &App) -> bool {
    !app.window.get_left_search_active() && app.window.get_left_search_query().is_empty()
}

/// After `what`: the quick search must be closed.
fn expect_closed(app: &mut App, what: &str, failures: &mut Vec<String>) {
    if !ps::wait(app, Duration::from_secs(5), search_closed) {
        failures.push(format!(
            "{what} left the quick search open: {:?}",
            search(app)
        ));
    }
}

/// Wait for the Copy dialog (its token) and return its summary.
fn copy_dialog(app: &mut App) -> Option<String> {
    ps::wait(app, Duration::from_secs(10), |app| {
        !app.window.get_copy_draft_token().is_empty()
    })
    .then(|| app.window.get_copy_draft_body().to_string())
}

/// Close the Copy dialog with Escape and wait until it is gone.
fn escape_copy(app: &mut App) {
    input::key(app.slint(), Key::Escape);
    ps::wait(app, Duration::from_secs(5), |app| {
        app.window.get_copy_draft_token().is_empty()
    });
}

/// A quick search never blocks a command (aac18026): F5, F6, F8, Ctrl+F8 and F4 close it
/// and act on the selection or the focused row the panel shows; Enter on a name that
/// matches nothing opens nothing and says "No name matches “…”"; a command pressed
/// while the search still computes acts on the row focused before; the panel filter
/// alone is never reported as the quick search.
fn commands(context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let folder = support::scratch(context, "search-commands")?;
    let target = support::scratch(context, "search-commands-target")?;
    let edited = support::scratch(context, "search-commands-edited")?;
    for (name, size) in [
        ("alpha.txt", 10),
        ("beta.txt", 20),
        ("gamma.txt", 30),
        ("zeta.dat", 40),
    ] {
        ps::write(&folder.join(name), vec![b'c'; size])?;
    }
    let cp = crate::tool::program(context, "cp")?;
    let copies = edited.display().to_string();
    let store = crate::options_support::SettingsStore::seeded(move |preferences| {
        preferences.editor = midnight_core::EditorPreferences {
            executable: Some(cp),
            arguments: vec!["{file}".into(), copies],
            rules: Vec::new(),
        };
    });
    let mut config = Config::new(&folder, &target);
    config.settings = Some(store);
    config.editor = true;
    let mut app = App::start(config);

    // F5 on a match: the dialog copies the match; Escape copies nothing.
    panel::select_row(&mut app, LEFT, "gamma.txt")?;
    type_search(&mut app, "be");
    expect_cursor(&app, "typing be", Some("beta.txt"), failures);
    input::key(app.slint(), Key::F5);
    match copy_dialog(&mut app) {
        None => failures.push("F5 while searching be opened no Copy dialog".into()),
        Some(body) => {
            if !body.contains("beta.txt") {
                failures.push(format!(
                    "F5 while searching be: the Copy dialog says {body:?}, not beta.txt"
                ));
            }
            expect_closed(&mut app, "F5", failures);
            escape_copy(&mut app);
        }
    }
    if !support::names_on_disk(&target).is_empty() {
        failures.push(format!(
            "Escape in the Copy dialog still copied {:?}",
            support::names_on_disk(&target)
        ));
    }

    // F6 on a match: the Move dialog moves the match; Escape moves nothing.
    panel::select_row(&mut app, LEFT, "beta.txt")?;
    type_search(&mut app, "ga");
    expect_cursor(&app, "typing ga", Some("gamma.txt"), failures);
    input::key(app.slint(), Key::F6);
    match copy_dialog(&mut app) {
        None => failures.push("F6 while searching ga opened no Move dialog".into()),
        Some(body) => {
            if !app.window.get_copy_draft_move() || !body.contains("gamma.txt") {
                failures.push(format!(
                    "F6 while searching ga: the dialog (move {}) says {body:?}, not a move of \
                     gamma.txt",
                    app.window.get_copy_draft_move()
                ));
            }
            expect_closed(&mut app, "F6", failures);
            escape_copy(&mut app);
        }
    }
    if !folder.join("gamma.txt").exists() || !support::names_on_disk(&target).is_empty() {
        failures.push(format!(
            "Escape in the Move dialog still moved: the target holds {:?}",
            support::names_on_disk(&target)
        ));
    }

    // F8 with no match: the confirmation names the focused shown row; Escape keeps it.
    panel::select_row(&mut app, LEFT, "gamma.txt")?;
    type_search(&mut app, "zz");
    expect_cursor(&app, "typing zz (no match)", Some("gamma.txt"), failures);
    input::key(app.slint(), Key::F8);
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        !app.window.get_operation_token().is_empty()
    }) {
        failures.push("F8 while searching zz opened no confirmation".into());
    } else {
        let body = format!(
            "{} / {}",
            app.window.get_operation_title(),
            app.window.get_operation_body()
        );
        if !body.contains("gamma.txt") {
            failures.push(format!(
                "F8 while searching zz asks {body:?}, not about the focused gamma.txt"
            ));
        }
        expect_closed(&mut app, "F8", failures);
        input::key(app.slint(), Key::Escape);
        // A confirmation left open could take a later Enter: stop here instead.
        if !ps::wait(&mut app, Duration::from_secs(5), |app| {
            app.window.get_operation_token().is_empty()
        }) {
            return Err("Escape did not close the F8 confirmation".into());
        }
    }
    if !folder.join("gamma.txt").exists() {
        failures.push("Escape in the F8 confirmation still deleted gamma.txt".into());
    }

    // Ctrl+F8 on a match: Checksums of the match alone.
    type_search(&mut app, "zet");
    expect_cursor(&app, "typing zet", Some("zeta.dat"), failures);
    let before = app.tool_windows().len();
    input::chord(app.slint(), &[Key::Control], Key::F8);
    if !ps::wait(&mut app, Duration::from_secs(10), |app| {
        app.tool_windows().len() > before
    }) {
        failures.push("Ctrl+F8 while searching zet opened no Checksums window".into());
    } else {
        expect_closed(&mut app, "Ctrl+F8", failures);
        let window = app.tool_windows().pop().expect("the Checksums window");
        crate::app::layout_tool(&window, 1000, 720);
        app.settle();
        let calculate = inspect::find_all(window.window(), AccessibleRole::Button, "Calculate")
            .pop()
            .ok_or("the Checksums window has no Calculate button")?;
        input::click_at(window.window(), inspect::center(&calculate));
        let texts = |window: &slint::Window| inspect::labels(window, AccessibleRole::Text);
        let shown = window.clone();
        if !ps::wait(&mut app, Duration::from_secs(20), |_| {
            texts(shown.window())
                .iter()
                .any(|text| text.starts_with("Completed · "))
        }) {
            failures.push(format!(
                "Checksums of the zet match never completed: {:?}",
                texts(window.window())
            ));
        }
        let rows = inspect::labels(window.window(), AccessibleRole::ListItem);
        let completed = texts(window.window())
            .into_iter()
            .find(|text| text.starts_with("Completed · "));
        if rows.len() != 1
            || !rows[0].starts_with("zeta.dat · ")
            || completed
                .as_deref()
                .is_none_or(|text| !text.starts_with("Completed · 1 calculated"))
        {
            failures.push(format!(
                "Checksums of the zet match: rows {rows:?}, status {completed:?}; expected \
                 zeta.dat alone"
            ));
        }
        input::key(window.window(), Key::Escape);
        if !ps::wait(&mut app, Duration::from_secs(10), |app| {
            app.tool_windows().len() == before
        }) {
            failures.push("Escape did not close the Checksums window".into());
        }
    }

    // Enter on a name that matches nothing: nothing opens; the status row says why.
    panel::select_row(&mut app, LEFT, "gamma.txt")?;
    type_search(&mut app, "zz");
    let listings = app.listings[0];
    let windows = app.tool_windows().len();
    input::key(app.slint(), Key::Return);
    app.settle();
    let note = "No name matches “zz”";
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        app.window.get_left_watch_status() == note
    }) {
        failures.push(format!(
            "Enter on zz: the status row says {:?}, expected {note:?}",
            app.window.get_left_watch_status()
        ));
    }
    expect_closed(&mut app, "Enter without a match", failures);
    expect_cursor(&app, "Enter without a match", Some("gamma.txt"), failures);
    if app.listings[0] != listings
        || app.tool_windows().len() != windows
        || support::shown_notice(&app).is_some()
        || app.location(LEFT).as_deref() != Some(folder.as_path())
    {
        failures.push(format!(
            "Enter on zz opened something: panel at {:?}, {} windows, notice {:?}",
            app.location(LEFT),
            app.tool_windows().len(),
            support::shown_notice(&app)
        ));
    }
    app.screenshot(
        context,
        "no-name-matches",
        "Enter on a name that matches nothing: the status row says so",
    );
    // Any key that moves the cursor ends the note.
    press(&mut app, Key::Home);
    if !app.window.get_left_watch_status().is_empty() {
        failures.push(format!(
            "the note stays after the cursor moved: {:?}",
            app.window.get_left_watch_status()
        ));
    }

    // F4 on a match: the editor runs on the match.
    type_search(&mut app, "al");
    expect_cursor(&app, "typing al", Some("alpha.txt"), failures);
    input::key(app.slint(), Key::F4);
    expect_closed(&mut app, "F4", failures);
    if !ps::wait(&mut app, Duration::from_secs(20), |_| {
        edited.join("alpha.txt").exists()
    }) {
        failures.push(format!(
            "F4 while searching al did not edit alpha.txt: the editor wrote {:?}",
            support::names_on_disk(&edited)
        ));
    }
    if support::names_on_disk(&edited) != ["alpha.txt"] {
        failures.push(format!(
            "F4 while searching al edited {:?}",
            support::names_on_disk(&edited)
        ));
    }

    // Marks win over the focused match: F5 copies the two marked files.
    for name in ["alpha.txt", "gamma.txt"] {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    app.settle();
    type_search(&mut app, "be");
    let run = support::transfer(&mut app, context, support::TransferSpec::copy())?;
    if let Err(error) = support::completed(&run, "F5 of two marks while searching be") {
        failures.push(error);
    }
    expect_closed(&mut app, "F5 of two marks", failures);
    if support::names_on_disk(&target) != ["alpha.txt", "gamma.txt"] {
        failures.push(format!(
            "F5 of the marks alpha.txt and gamma.txt while searching be copied {:?}",
            support::names_on_disk(&target)
        ));
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // A command pressed before the search has its matches acts on the row focused
    // before: the search is computed in the background (its request leaves with the
    // next frame), F5 cancels it, and its late result cannot move the cursor (aac18026).
    panel::select_row(&mut app, LEFT, "gamma.txt")?;
    input::type_text(app.slint(), "zet");
    input::key(app.slint(), Key::F5);
    match copy_dialog(&mut app) {
        None => failures.push("F5 right after typing zet opened no Copy dialog".into()),
        Some(body) => {
            if !body.contains("gamma.txt") {
                failures.push(format!(
                    "F5 pressed before the search for zet had its matches: the Copy dialog \
                     says {body:?}, expected the row focused before, gamma.txt"
                ));
            }
            escape_copy(&mut app);
            app.idle(Duration::from_millis(200));
            if ps::cursor(&app, LEFT).as_deref() != Some("gamma.txt") || !search_closed(&app) {
                failures.push(format!(
                    "after F5 cancelled the search for zet the cursor is on {:?} and the \
                     search {:?}; its late result must not move the cursor",
                    ps::cursor(&app, LEFT),
                    search(&app)
                ));
            }
        }
    }

    // The panel filter alone is not the quick search: the status row stays empty.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    if !ps::wait(&mut app, Duration::from_secs(5), filter_open) {
        return Err("Ctrl+F12 opened no Panel filter".into());
    }
    crate::options_support::choose(&mut app, "Filter mode", "Masks")?;
    input::fill(app.slint(), "File filter expression", "*.txt")?;
    ok(&mut app)?;
    if !lists(&mut app, LEFT, &["alpha.txt", "beta.txt", "gamma.txt"]) {
        failures.push(format!(
            "the *.txt filter lists {:?}",
            ps::top_level(&app, LEFT)
        ));
    }
    // A command with the filter on acts on the focused shown row; nothing is noted.
    panel::select_row(&mut app, LEFT, "beta.txt")?;
    input::key(app.slint(), Key::F5);
    match copy_dialog(&mut app) {
        None => failures.push("F5 with only the panel filter opened no Copy dialog".into()),
        Some(body) => {
            if !body.contains("beta.txt") {
                failures.push(format!(
                    "F5 with only the panel filter: the Copy dialog says {body:?}"
                ));
            }
            escape_copy(&mut app);
        }
    }
    let status = app.window.get_left_watch_status().to_string();
    if !status.is_empty() || app.window.get_left_search_active() {
        failures.push(format!(
            "with only the panel filter applied the status row says {status:?} (search {:?})",
            search(&app)
        ));
    }
    open_filter(&mut app, LEFT)?;
    input::click_button(app.slint(), "Show all")?;
    ps::wait(&mut app, Duration::from_secs(5), |app| !filter_open(app));
    app.shutdown()
}
