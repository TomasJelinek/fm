//! The Times page of Ctrl+F2 driven like a user does: Up/Down steppers on each part of
//! the date, time and UTC offset fields; the calendar (Alt+Down; Left/Right a day,
//! Up/Down a week, Page Up/Down a month, Enter) and its Previous/Next month, day and
//! Today buttons; the time picker (Alt+Down; Left/Right a column, Up/Down by one, Page
//! Up/Down by ten, Escape) and its ▲/▼ buttons; Now. Every picked value is applied and
//! read back from disk to the second. Escape in a picker closes only the picker, the next
//! one closes the editor without applying anything. Ctrl+F7 opens the same editor on
//! its Case page and renames a file to lower case. The finer points: Tab alone leaves a
//! row unticked, Escape in a picker restores the time and the tick, the picker's
//! Home/End/Tab/digits/Space, the wheel, the calendar's year and month-end keys, an
//! outside click, and an unticked row that keeps its value and is not applied.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, inspect, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use i_slint_core::items::AccessibleRole;
use slint::{LogicalPosition, platform::Key};
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
    for name in ["one.txt", "two.txt", "three.txt", "details.txt"] {
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
    picker_details(&mut app, context, &folder, &mut failures)?;
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
    // by ten; Enter closes it with the changed time kept, and Escape restores the time it
    // opened with.
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
        press(app, Key::Return, false);
        if popups(app) != before {
            failures.push("Enter did not close the time picker".into());
        }
        if app.window.get_metadata_token().is_empty() {
            failures.push("Enter in the time picker closed the editor".into());
        }
        expect(
            failures,
            "the time after the picker keys",
            Some(field(app, TIME)),
            "07:05:07",
        );
        // Escape closes only the picker and restores the time it opened with.
        press(app, Key::DownArrow, true);
        if popups(app) != before + 1 {
            failures.push("Alt+Down opened no time picker the second time".into());
        } else {
            press(app, Key::UpArrow, false);
            expect(
                failures,
                "time picker Up before Escape",
                spin(app, "Modified hour"),
                "08",
            );
            press(app, Key::Escape, false);
            if popups(app) != before {
                failures.push("Escape did not close the time picker".into());
            }
            if app.window.get_metadata_token().is_empty() {
                failures.push("Escape in the time picker closed the editor".into());
            }
            expect(
                failures,
                "the time after Escape in the picker",
                Some(field(app, TIME)),
                "07:05:07",
            );
        }
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

/// The accessible label of the control that has the keyboard, if any.
fn focused(app: &App) -> Option<String> {
    let item = i_slint_core::window::WindowInner::from_pub(app.slint())
        .focus_item
        .borrow()
        .upgrade()?;
    inspect::label(&item)
}

/// Press Tab until the control labelled `label` has the keyboard (at most `limit`
/// presses); whether it got it.
fn tab_to(app: &mut App, label: &str, limit: usize) -> bool {
    for _ in 0..limit {
        if focused(app).as_deref() == Some(label) {
            return true;
        }
        press(app, Key::Tab, false);
    }
    focused(app).as_deref() == Some(label)
}

/// Turn the wheel by `delta` logical pixels over the middle of `item`.
fn wheel_over(app: &mut App, item: &i_slint_core::item_tree::ItemRc, delta: f32) {
    let position = inspect::center(item);
    input::move_to(app.slint(), position);
    app.slint()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: delta,
        });
    app.settle();
}

/// The Times page's finer points (1e8a987f): Tab alone leaves a row unticked and Up in
/// its field ticks it; Escape in the time picker restores the time and the tick; the
/// picker's Home/End, Tab and Shift+Tab (round the columns), typed digits that move on,
/// Space keeping the time; the wheel (one step per 60 px notch, fine deltas added up;
/// over the calendar it turns months); the calendar's Ctrl+Page Up/Down (a year),
/// Home/End (first and last day) and Space; an outside click keeping the change; an
/// unticked row keeping its value and leaving the disk alone, ticked again applying it.
/// Holding ▲ (a 500 ms delay, then every 50 ms) needs the controllable clock: SKIP.
fn picker_details(
    app: &mut App,
    context: &mut Context,
    folder: &std::path::Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let path = folder.join("details.txt");
    let before = modified(&path);
    panel::select_row(app, LEFT, "details.txt")?;
    ps::open_metadata(app, false)?;
    input::click_button(app.slint(), "Times")?;
    app.settle();
    let editor = popups(app);
    let ticked = |app: &App| ps::checked(app, "Change Modified");

    // Tab alone only moves the keyboard; Up in the date field ticks the row.
    if !tab_to(app, DATE, 24) {
        failures.push(format!(
            "Tab never reached the Modified date field (the keyboard is on {:?})",
            focused(app)
        ));
    } else {
        if ticked(app) != Some(false) {
            failures.push(format!(
                "Tab into the Modified date ticked the row: {:?}",
                ticked(app)
            ));
        }
        let date = field(app, DATE);
        press(app, Key::UpArrow, false);
        if ticked(app) != Some(true) || field(app, DATE) == date {
            failures.push(format!(
                "Up in the Modified date: {date:?} → {:?}, row ticked {:?}",
                field(app, DATE),
                ticked(app)
            ));
        }
    }

    // A known time to start from; untick the row again by its box.
    input::fill(app.slint(), TIME, "10:20:00")?;
    app.settle();
    input::click(app.slint(), AccessibleRole::Checkbox, "Change Modified")?;
    app.settle();
    if ticked(app) != Some(false) {
        failures.push("clicking the ticked Change Modified box did not untick it".into());
    }
    // Escape in the picker restores the time and the tick it opened with.
    if !tab_to(app, TIME, 24) {
        failures.push(format!(
            "Tab never reached the Modified time field (the keyboard is on {:?})",
            focused(app)
        ));
    }
    press(app, Key::DownArrow, true);
    if popups(app) != editor + 1 || spin(app, "Modified hour").is_none() {
        failures.push("Alt+Down in the time field opened no picker".into());
    } else {
        press(app, Key::UpArrow, false);
        if field(app, TIME) != "11:20:00" || ticked(app) != Some(true) {
            failures.push(format!(
                "Up in the picker: the time is {:?} and the row ticked {:?}, expected 11:20:00 \
                 ticked",
                field(app, TIME),
                ticked(app)
            ));
        }
        press(app, Key::Escape, false);
        if field(app, TIME) != "10:20:00" || ticked(app) != Some(false) {
            failures.push(format!(
                "Escape in the picker left {:?} with the row ticked {:?}, expected 10:20:00 \
                 unticked as it opened",
                field(app, TIME),
                ticked(app)
            ));
        }
    }

    // The picker's keys: Home/End, Tab and Shift+Tab round the columns, digits, Space.
    press(app, Key::DownArrow, true);
    if popups(app) != editor + 1 {
        failures.push("Alt+Down opened no time picker for its keys".into());
    } else {
        for (key, shift, column, wanted) in [
            (Key::Home, false, "Modified hour", "00"),
            (Key::End, false, "Modified hour", "23"),
            (Key::Tab, false, "Modified minute", "20"),
            (Key::Tab, true, "Modified hour", "23"),
            (Key::Tab, true, "Modified second", "00"),
            (Key::Tab, false, "Modified hour", "23"),
        ] {
            if shift {
                input::chord(app.slint(), &[Key::Shift], key);
                app.settle();
            } else {
                press(app, key, false);
            }
            expect(
                failures,
                &format!("time picker {}{key:?}", if shift { "Shift+" } else { "" }),
                spin(app, column),
                wanted,
            );
        }
        // Digits set the hour as typed and move on; one digit sets a minute of 07.
        input::type_text(app.slint(), "15");
        app.settle();
        // A complete hour moves the picker on: its minute column is chosen now.
        if !field(app, TIME).starts_with("15:") {
            failures.push(format!(
                "typing 1 5 in the hour gave the time {:?}",
                field(app, TIME)
            ));
        }
        if spin(app, "Modified minute").is_none() {
            failures.push("after a complete hour the picker did not move to the minute".into());
        }
        input::type_text(app.slint(), "7");
        app.settle();
        expect(
            failures,
            "typing 7 in the minute",
            Some(field(app, TIME)),
            "15:07:00",
        );
        press(app, Key::Space, false);
        if popups(app) != editor {
            failures.push("Space did not close the time picker".into());
        }
        expect(
            failures,
            "the time Space kept",
            Some(field(app, TIME)),
            "15:07:00",
        );
    }

    // The wheel over a column: one step per 60 px notch, fine deltas added up.
    if input::click_button(app.slint(), "Modified: choose time").is_err() {
        failures.push("no Modified: choose time button".into());
    } else {
        app.settle();
        match inspect::find(app.slint(), AccessibleRole::Button, "Increase Hour") {
            None => failures.push("the time picker shows no Increase Hour arrow".into()),
            Some(arrow) => {
                // The middle of the hour column: below its ▲.
                let (x, y, w, h) = inspect::rect(&arrow);
                let column = LogicalPosition::new(x + w / 2.0, y + h * 1.5);
                input::move_to(app.slint(), column);
                for delta in [60.0, 30.0, 30.0, -60.0, 20.0] {
                    app.slint()
                        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                            position: column,
                            delta_x: 0.0,
                            delta_y: delta,
                        });
                    app.settle();
                }
                expect(
                    failures,
                    "the wheel over the hour (+60, +30, +30, −60, +20 px)",
                    spin(app, "Modified hour"),
                    "16",
                );
            }
        }
        // A click outside it (on the date field of the editor below) closes the picker
        // and keeps the change.
        match inspect::find_all(app.slint(), AccessibleRole::TextInput, DATE).first() {
            None => failures.push("no Modified date field to click outside the picker".into()),
            Some(date) => {
                input::click_at(app.slint(), inspect::center(date));
                app.settle();
            }
        }
        if popups(app) != editor {
            failures.push("a click outside did not close the time picker".into());
        }
        expect(
            failures,
            "the time an outside click kept",
            Some(field(app, TIME)),
            "16:07:00",
        );
    }

    // The calendar: Ctrl+Page Up/Down a year, Home/End the month's ends, Space chooses;
    // the wheel turns months.
    input::fill(app.slint(), DATE, "2001-02-03")?;
    app.settle();
    press(app, Key::DownArrow, true);
    if popups(app) != editor + 1 || spin(app, CALENDAR).is_none() {
        failures.push("Alt+Down in the date field opened no calendar".into());
    } else {
        for (control, key, wanted) in [
            (true, Key::PageDown, "2002-02-03"),
            (true, Key::PageUp, "2001-02-03"),
            (false, Key::End, "2001-02-28"),
            (false, Key::Home, "2001-02-01"),
        ] {
            if control {
                input::chord(app.slint(), &[Key::Control], key);
                app.settle();
            } else {
                press(app, key, false);
            }
            expect(
                failures,
                &format!("calendar {}{key:?}", if control { "Ctrl+" } else { "" }),
                spin(app, CALENDAR),
                wanted,
            );
        }
        if let Some(calendar) = inspect::find(app.slint(), AccessibleRole::Spinbox, CALENDAR) {
            // Up goes back a month per 60 px notch.
            wheel_over(app, &calendar, 60.0);
            expect(
                failures,
                "the wheel up over the calendar",
                spin(app, CALENDAR),
                "2001-01-01",
            );
            wheel_over(app, &calendar, -30.0);
            wheel_over(app, &calendar, -30.0);
            expect(
                failures,
                "two half notches down over the calendar",
                spin(app, CALENDAR),
                "2001-02-01",
            );
        }
        press(app, Key::Space, false);
        if popups(app) != editor {
            failures.push("Space did not choose the calendar's day".into());
        }
        expect(
            failures,
            "the day Space chose",
            Some(field(app, DATE)),
            "2001-02-01",
        );
    }

    // An unticked row keeps what it shows and is not applied; ticked again, it is.
    input::fill(app.slint(), DATE, "2001-02-03")?;
    input::fill(app.slint(), TIME, "04:05:06")?;
    input::fill(app.slint(), OFFSET, "Z")?;
    app.settle();
    if ticked(app) == Some(true) {
        input::click(app.slint(), AccessibleRole::Checkbox, "Change Modified")?;
        app.settle();
    }
    if ticked(app) != Some(false) || field(app, DATE) != "2001-02-03" {
        failures.push(format!(
            "unticking the row: ticked {:?}, date {:?}, expected the shown 2001-02-03 kept",
            ticked(app),
            field(app, DATE)
        ));
    }
    // Apply with nothing ticked changes nothing; ticking Accessed gives it a job.
    input::click(app.slint(), AccessibleRole::Checkbox, "Change Accessed")?;
    app.settle();
    match ps::apply_metadata(app) {
        Err(error) => failures.push(format!("Apply with Modified unticked: {error}")),
        Ok(_) => {
            if modified(&path) != before {
                failures.push(format!(
                    "Apply with the Modified row unticked changed details.txt: {before:?} → {:?}",
                    modified(&path)
                ));
            }
        }
    }
    panel::select_row(app, LEFT, "details.txt")?;
    ps::open_metadata(app, false)?;
    input::click_button(app.slint(), "Times")?;
    app.settle();
    input::fill(app.slint(), DATE, "2001-02-03")?;
    input::fill(app.slint(), TIME, "04:05:06")?;
    input::fill(app.slint(), OFFSET, "Z")?;
    app.settle();
    if ticked(app) != Some(true) {
        input::click(app.slint(), AccessibleRole::Checkbox, "Change Modified")?;
        app.settle();
    }
    match ps::apply_metadata(app) {
        Err(error) => failures.push(format!("Apply with Modified ticked again: {error}")),
        Ok(_) => {
            let wanted = ps::instant((2001, 2, 3), (4, 5, 6), 0, 0);
            if modified(&path) != Some(wanted) {
                failures.push(format!(
                    "Apply with the row ticked again: details.txt modified {:?}, expected \
                     2001-02-03 04:05:06 UTC",
                    modified(&path)
                ));
            }
        }
    }
    context.note(
        "SKIP: holding the time picker's ▲ (+10 in 1,000 ms after the 500 ms delay, +1 in \
         300 ms, paused off the arrow): needs the scenario's own clock (harness H1)",
    );
    Ok(())
}
