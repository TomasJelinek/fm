//! The Times page of Ctrl+F2 driven like a user does: Up/Down steppers on each part of
//! the date, time and UTC offset fields; the calendar (Alt+Down; Left/Right a day,
//! Up/Down a week, Page Up/Down a month, Enter) and its Previous/Next month, day and
//! Today buttons; the time picker (Alt+Down; Left/Right a column, Up/Down by one, Page
//! Up/Down by ten, Escape) and its ▲/▼ buttons; Now. Every picked value is applied and
//! read back from disk to the second. Escape in a picker closes only the picker, the next
//! one closes the editor without applying anything. Ctrl+F7 opens the same editor on
//! its Case page and renames a file to lower case.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use i_slint_core::items::AccessibleRole;
use slint::platform::Key;
use std::{
    fs,
    time::{Duration, SystemTime},
};

const DATE: &str = "Modified date";
const TIME: &str = "Modified time";
const OFFSET: &str = "Modified UTC offset";
const CALENDAR: &str = "Modified calendar date";

fn field(app: &App, label: &str) -> String {
    ps::value(app, AccessibleRole::TextInput, label).unwrap_or_default()
}

fn spin(app: &App, label: &str) -> Option<String> {
    ps::value(app, AccessibleRole::Spinbox, label)
}

/// Focus a field by clicking it, put the cursor at its start (`home`) or end, and
/// press Up or Down; the field's text afterwards.
fn step(app: &mut App, label: &str, home: bool, rights: usize, up: bool) -> Result<String, String> {
    input::click(app.slint(), AccessibleRole::TextInput, label)?;
    input::key(app.slint(), if home { Key::Home } else { Key::End });
    for _ in 0..rights {
        input::key(app.slint(), Key::RightArrow);
    }
    input::key(app.slint(), if up { Key::UpArrow } else { Key::DownArrow });
    app.settle();
    Ok(field(app, label))
}

fn expect(failures: &mut Vec<String>, what: &str, found: Option<String>, wanted: &str) {
    if found.as_deref() != Some(wanted) {
        failures.push(format!("{what}: {found:?}, expected {wanted:?}"));
    }
}

/// The number of popups open on the main window (the editor is one).
fn popups(app: &App) -> usize {
    inspect::roots(app.slint()).len() - 1
}

/// Press `key` (with Alt when `alt`) in the focused control and settle.
fn press(app: &mut App, key: Key, alt: bool) {
    if alt {
        input::chord(app.slint(), &[Key::Alt], key);
    } else {
        input::key(app.slint(), key);
    }
    app.settle();
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

pub fn times(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "times")?;
    for name in ["one.txt", "two.txt", "three.txt"] {
        ps::write(&folder.join(name), name)?;
    }
    ps::write(&folder.join("MiXeD Name.TXT"), "case")?;
    ps::folder(&folder.join("tree/nested"))?;
    ps::write(&folder.join("tree/inner.txt"), "inner")?;
    ps::write(&folder.join("tree/nested/inner2.txt"), "inner2")?;
    let mut app = App::start(Config::new(&folder, &context.work));
    let mut failures = Vec::new();
    pickers(&mut app, context, &folder, &mut failures)?;
    now(&mut app, context, &folder, &mut failures)?;
    escape(&mut app, &folder, &mut failures)?;
    case(&mut app, context, &folder, &mut failures)?;
    recursive(&mut app, &folder, &mut failures)?;
    app.shutdown()?;
    support::report(&failures)
}

/// Steppers, the calendar and the time picker on one.txt, applied and read back.
fn pickers(
    app: &mut App,
    context: &mut Context,
    folder: &std::path::Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::select_row(app, LEFT, "one.txt")?;
    ps::open_metadata(app, false)?;
    input::click_button(app.slint(), "Times")?;
    app.settle();
    if ps::checked(app, "Change Modified") != Some(false) {
        failures.push(format!(
            "the Modified row starts {:?}, expected unticked",
            ps::checked(app, "Change Modified")
        ));
    }
    input::fill(app.slint(), DATE, "2021-03-04")?;
    input::fill(app.slint(), TIME, "05:06:07")?;
    input::fill(app.slint(), OFFSET, "+02:00")?;
    app.settle();
    if ps::checked(app, "Change Modified") != Some(true) {
        failures.push("typing into the Modified fields did not tick its row".into());
    }

    // Up/Down change the part under the cursor.
    for (label, home, rights, up, wanted) in [
        (DATE, true, 0, true, "2022-03-04"),
        (DATE, false, 0, false, "2022-03-03"),
        (DATE, true, 5, true, "2022-04-03"),
        (TIME, true, 0, true, "06:06:07"),
        (TIME, false, 0, true, "06:06:08"),
        (TIME, true, 3, false, "06:05:08"),
        (OFFSET, true, 0, true, "+03:00"),
        (OFFSET, false, 0, true, "+03:15"),
    ] {
        let found = step(app, label, home, rights, up)?;
        expect(
            failures,
            &format!(
                "{label}: {} at {} {}",
                if up { "Up" } else { "Down" },
                if home { "Home" } else { "End" },
                if rights > 0 {
                    format!("+{rights} Right")
                } else {
                    String::new()
                }
            ),
            Some(found),
            wanted,
        );
    }

    // The calendar: Alt+Down in the date field, keys move its cursor, Enter chooses.
    input::click(app.slint(), AccessibleRole::TextInput, DATE)?;
    let before = popups(app);
    press(app, Key::DownArrow, true);
    if popups(app) != before + 1 || spin(app, CALENDAR).is_none() {
        failures.push(format!(
            "Alt+Down in the date field opened no calendar (popups {before} → {})",
            popups(app)
        ));
    } else {
        expect(
            failures,
            "the calendar opens on the field's date",
            spin(app, CALENDAR),
            "2022-04-03",
        );
        app.screenshot(context, "calendar", "The Modified date calendar (Alt+Down)");
        for (key, wanted) in [
            (Key::RightArrow, "2022-04-04"),
            (Key::RightArrow, "2022-04-05"),
            (Key::DownArrow, "2022-04-12"),
            (Key::PageDown, "2022-05-12"),
            (Key::UpArrow, "2022-05-05"),
            (Key::LeftArrow, "2022-05-04"),
            (Key::PageUp, "2022-04-04"),
            (Key::PageDown, "2022-05-04"),
        ] {
            press(app, key, false);
            expect(
                failures,
                &format!("calendar {key:?}"),
                spin(app, CALENDAR),
                wanted,
            );
        }
        press(app, Key::Return, false);
        if popups(app) != before {
            failures.push("Enter in the calendar did not close it".into());
        }
        expect(
            failures,
            "the date chosen with Enter",
            Some(field(app, DATE)),
            "2022-05-04",
        );
        if app.window.get_metadata_token().is_empty() {
            failures.push("choosing a date closed the editor".into());
        }
    }

    // Its buttons: Next month twice, Previous month once, then a day.
    if let Err(error) = input::click_button(app.slint(), "Modified: choose date") {
        failures.push(format!("calendar button: {error}"));
    } else {
        app.settle();
        for button in ["Next month", "Next month", "Previous month"] {
            if let Err(error) = ps::click_popup_button(app, button) {
                failures.push(format!("calendar {button}: {error}"));
            }
        }
        expect(
            failures,
            "the calendar after Next, Next, Previous month",
            spin(app, CALENDAR),
            "2022-06-04",
        );
        match ps::click_popup_button(app, "2022-06-15") {
            Err(error) => failures.push(format!("calendar day 15: {error}")),
            Ok(()) => {
                expect(
                    failures,
                    "the date picked by its day button",
                    Some(field(app, DATE)),
                    "2022-06-15",
                );
                if popups(app) != before {
                    failures.push("picking a day did not close the calendar".into());
                }
            }
        }
    }

    // The time picker: Alt+Down, Left/Right choose a column, Up/Down by one, Page Up/Down
    // by ten, Escape closes it with the changed time kept.
    input::click(app.slint(), AccessibleRole::TextInput, TIME)?;
    press(app, Key::DownArrow, true);
    if popups(app) != before + 1 || spin(app, "Modified hour").is_none() {
        failures.push("Alt+Down in the time field opened no time picker".into());
    } else {
        app.screenshot(
            context,
            "time-picker",
            "The Modified time picker (Alt+Down)",
        );
        for (key, label, wanted) in [
            (Key::UpArrow, "Modified hour", "07"),
            (Key::RightArrow, "Modified minute", "05"),
            (Key::PageUp, "Modified minute", "15"),
            (Key::RightArrow, "Modified second", "08"),
            (Key::DownArrow, "Modified second", "07"),
            (Key::LeftArrow, "Modified minute", "15"),
            (Key::PageDown, "Modified minute", "05"),
        ] {
            press(app, key, false);
            expect(
                failures,
                &format!("time picker {key:?}"),
                spin(app, label),
                wanted,
            );
        }
        press(app, Key::Escape, false);
        if popups(app) != before {
            failures.push("Escape did not close the time picker".into());
        }
        if app.window.get_metadata_token().is_empty() {
            failures.push("Escape in the time picker closed the editor".into());
        }
        expect(
            failures,
            "the time after the picker keys",
            Some(field(app, TIME)),
            "07:05:07",
        );
    }
    // Its ▲/▼ buttons; Enter closes it.
    if let Err(error) = input::click_button(app.slint(), "Modified: choose time") {
        failures.push(format!("time picker button: {error}"));
    } else {
        app.settle();
        for button in [
            "Increase Second",
            "Increase Second",
            "Decrease Hour",
            "Increase Minute",
        ] {
            if let Err(error) = ps::click_popup_button(app, button) {
                failures.push(format!("time picker {button}: {error}"));
            }
        }
        expect(
            failures,
            "the time after the ▲/▼ buttons",
            Some(field(app, TIME)),
            "06:06:09",
        );
        press(app, Key::Return, false);
        if popups(app) != before {
            failures.push("Enter did not close the time picker".into());
        }
    }

    // Applied, the file has exactly the picked moment.
    let accessed = fs::metadata(folder.join("one.txt"))
        .and_then(|metadata| metadata.accessed())
        .ok();
    let wanted = ps::instant((2022, 6, 15), (6, 6, 9), 0, 3 * 60 + 15);
    match ps::apply_metadata(app) {
        Err(error) => failures.push(format!("applying the picked time: {error}")),
        Ok(_) => {
            let found = modified(&folder.join("one.txt"));
            if found != Some(wanted) {
                failures.push(format!(
                    "one.txt modified {found:?}, expected the picked 2022-06-15 06:06:09 +03:15 ({wanted:?})"
                ));
            }
            let accessed_now = fs::metadata(folder.join("one.txt"))
                .and_then(|metadata| metadata.accessed())
                .ok();
            if accessed_now != accessed {
                failures.push(format!(
                    "the unticked Accessed time changed: {accessed:?} → {accessed_now:?}"
                ));
            }
        }
    }
    Ok(())
}

/// Parse the shown date, time and offset when they use the year-month-day and 24-hour
/// forms: the instant they name.
fn shown_instant(date: &str, time: &str, offset: &str) -> Option<SystemTime> {
    let numbers = |text: &str, separator: char| -> Option<Vec<i64>> {
        text.split(separator)
            .map(|part| part.trim().parse::<i64>().ok())
            .collect()
    };
    let date = numbers(date, '-').filter(|parts| parts.len() == 3 && parts[0] > 999)?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let clock = numbers(clock, ':').filter(|parts| parts.len() == 3)?;
    let nanos = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<9}").get(..9)?.parse::<u32>().ok()?
    };
    let minutes = if offset == "Z" {
        0
    } else {
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let parts = numbers(offset.trim_start_matches(['+', '-']), ':')?;
        sign * (parts.first()? * 60 + parts.get(1).copied().unwrap_or(0))
    };
    Some(ps::instant(
        (date[0], date[1], date[2]),
        (clock[0], clock[1], clock[2]),
        nanos,
        minutes,
    ))
}

/// Now fills the row with the current moment and ticks it; Apply sets exactly that
/// moment. Today in the calendar is Now's date.
fn now(
    app: &mut App,
    context: &mut Context,
    folder: &std::path::Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::select_row(app, LEFT, "two.txt")?;
    ps::open_metadata(app, false)?;
    input::click_button(app.slint(), "Times")?;
    app.settle();
    let pressed = SystemTime::now();
    input::click_button(app.slint(), "Modified: Now")?;
    app.settle();
    let (date, time, offset) = (field(app, DATE), field(app, TIME), field(app, OFFSET));
    if ps::checked(app, "Change Modified") != Some(true) {
        failures.push("Now did not tick the Modified row".into());
    }
    if date.is_empty() || time.is_empty() || offset.is_empty() {
        failures.push(format!(
            "Now left a field empty: date {date:?}, time {time:?}, offset {offset:?}"
        ));
    }
    app.screenshot(context, "now", "Modified filled in by Now");
    // Today picks Now's date.
    if input::click_button(app.slint(), "Modified: choose date").is_ok() {
        app.settle();
        if let Err(error) = ps::click_popup_button(app, "Today") {
            failures.push(format!("calendar Today: {error}"));
        } else if field(app, DATE) != date {
            failures.push(format!(
                "Today picked {:?}, Now filled in {date:?}",
                field(app, DATE)
            ));
        }
    } else {
        failures.push("no Modified: choose date button".into());
    }
    let shown = shown_instant(&date, &time, &offset);
    match ps::apply_metadata(app) {
        Err(error) => failures.push(format!("applying Now: {error}")),
        Ok(_) => {
            let found = modified(&folder.join("two.txt"));
            let distance = found.and_then(|found| {
                found
                    .duration_since(pressed)
                    .or_else(|error| Ok::<_, ()>(error.duration()))
                    .ok()
            });
            if distance.is_none_or(|distance| distance > Duration::from_secs(5)) {
                failures.push(format!(
                    "two.txt modified {found:?} after Now at {pressed:?}: not the current moment"
                ));
            }
            // Now keeps the exact moment it took; the fields show it to the second.
            let within = |shown: SystemTime| {
                found
                    .and_then(|found| found.duration_since(shown).ok())
                    .is_some_and(|after| after < Duration::from_secs(1))
            };
            match shown {
                Some(shown) if !within(shown) => failures.push(format!(
                    "Apply did not set the moment Now showed ({date} {time} {offset} = {shown:?}, to the second): two.txt modified {found:?}"
                )),
                Some(shown) => context.note(format!(
                    "Now showed {date} {time} {offset}; Apply set {:?} after that second",
                    found.and_then(|found| found.duration_since(shown).ok())
                )),
                None => context.note(format!(
                    "Now's fields use another form: {date:?} {time:?} {offset:?}; checked against the clock only"
                )),
            }
        }
    }
    Ok(())
}

/// Escape in a picker closes only the picker; the next Escape closes the editor and
/// nothing is applied.
fn escape(
    app: &mut App,
    folder: &std::path::Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let path = folder.join("three.txt");
    let before = modified(&path);
    panel::select_row(app, LEFT, "three.txt")?;
    ps::open_metadata(app, false)?;
    input::click_button(app.slint(), "Times")?;
    input::fill(app.slint(), DATE, "2000-01-01")?;
    app.settle();
    let editor = popups(app);
    press(app, Key::DownArrow, true);
    if popups(app) != editor + 1 {
        failures.push("Alt+Down after typing opened no calendar".into());
    }
    press(app, Key::Escape, false);
    if popups(app) != editor || app.window.get_metadata_token().is_empty() {
        failures.push(format!(
            "Escape in the calendar: popups {} (editor alone is {editor}), editor open {}",
            popups(app),
            !app.window.get_metadata_token().is_empty()
        ));
    }
    if field(app, DATE) != "2000-01-01" {
        failures.push(format!(
            "Escape in the calendar changed the date to {:?}",
            field(app, DATE)
        ));
    }
    press(app, Key::Escape, false);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.window.get_metadata_token().is_empty()
    }) {
        failures.push("the second Escape did not close the editor".into());
        support::dismiss_all(app);
    } else if let Err(error) = ps::stays_idle(app, Duration::from_millis(500)) {
        failures.push(format!("after Escape closed the editor: {error}"));
    }
    if modified(&path) != before {
        failures.push(format!(
            "Escape applied the typed time: three.txt modified {:?} → {:?}",
            before,
            modified(&path)
        ));
    }
    Ok(())
}

/// Ctrl+F7 opens the editor on its Case page with the conversion ticked; lowercase of
/// the whole name renames the file, listed with the cursor on it.
fn case(
    app: &mut App,
    context: &mut Context,
    folder: &std::path::Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::select_row(app, LEFT, "MiXeD Name.TXT")?;
    ps::open_metadata(app, true)?;
    if ps::checked(app, "Change filename case") != Some(true) {
        failures.push(format!(
            "Ctrl+F7 opened with Change filename case {:?}, expected ticked on the Case page",
            ps::checked(app, "Change filename case")
        ));
        input::click_button(app.slint(), "Case")?;
        input::click(
            app.slint(),
            AccessibleRole::Checkbox,
            "Change filename case",
        )?;
        app.settle();
    }
    support::choose(app, "Conversion", "lowercase", 4)?;
    support::choose(app, "Apply to", "Whole filename", 3)?;
    app.screenshot(context, "case", "Ctrl+F7 on the Case page");
    match ps::apply_metadata(app) {
        Err(error) => failures.push(format!("Ctrl+F7 lowercase: {error}")),
        Ok(_) => {
            let names = support::names_on_disk(folder);
            if !names.contains(&"mixed name.txt".to_owned())
                || names.contains(&"MiXeD Name.TXT".to_owned())
            {
                failures.push(format!("Ctrl+F7 lowercase left the folder as {names:?}"));
            } else if !ps::wait(app, Duration::from_secs(5), |app| {
                ps::shown(app, LEFT).contains(&"mixed name.txt".to_owned())
            }) {
                failures.push("the lower-cased name is not listed".into());
            }
        }
    }
    Ok(())
}

/// Include subfolders applies the time to a folder and everything below it; Back to
/// editing from the review keeps the draft.
fn recursive(
    app: &mut App,
    folder: &std::path::Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::select_row(app, LEFT, "tree")?;
    ps::open_metadata(app, false)?;
    input::click_button(app.slint(), "Times")?;
    input::fill(app.slint(), DATE, "2015-05-05")?;
    input::fill(app.slint(), TIME, "05:05:05")?;
    input::fill(app.slint(), OFFSET, "Z")?;
    input::click(app.slint(), AccessibleRole::Checkbox, "Include subfolders")?;
    app.settle();
    if ps::checked(app, "Include subfolders") != Some(true) {
        failures.push("Include subfolders did not tick".into());
    }
    input::click_button(app.slint(), "Review changes")?;
    if !ps::wait(app, Duration::from_secs(20), |app| {
        inspect::find(app.slint(), AccessibleRole::Button, "Back to editing").is_some()
    }) {
        failures.push("Review changes offered no Back to editing".into());
    } else {
        input::click_button(app.slint(), "Back to editing")?;
        app.settle();
        if field(app, DATE) != "2015-05-05" || ps::checked(app, "Include subfolders") != Some(true)
        {
            failures.push(format!(
                "Back to editing lost the draft: date {:?}, Include subfolders {:?}",
                field(app, DATE),
                ps::checked(app, "Include subfolders")
            ));
        }
    }
    let wanted = ps::instant((2015, 5, 5), (5, 5, 5), 0, 0);
    match ps::apply_metadata(app) {
        Err(error) => failures.push(format!("Include subfolders: {error}")),
        Ok(_) => {
            for relative in [
                "tree",
                "tree/inner.txt",
                "tree/nested",
                "tree/nested/inner2.txt",
            ] {
                let found = modified(&folder.join(relative));
                if found != Some(wanted) {
                    failures.push(format!(
                        "Include subfolders left {relative} modified {found:?}, expected 2015-05-05 05:05:05 Z"
                    ));
                }
            }
        }
    }
    Ok(())
}
