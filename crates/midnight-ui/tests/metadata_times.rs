//! The Times page with real presenter callbacks: typed dates and times, arrow-key
//! steps and both pickers reach the exact reviewed patch, and the header is plain.
#[path = "support/metadata_form.rs"]
mod metadata_form;
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use metadata_form::{Editor, at, calendar, format};
use midnight_core::*;
use slint::platform::Key;
use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn typed_date_time_and_offset_with_an_arrow_step_apply_the_exact_instant() {
    let mut editor = Editor::open();
    let header = editor.texts();
    // The single item's name is shown in left-to-right isolation marks.
    assert!(
        header
            .iter()
            .any(|text| text == "\u{2066}report.txt\u{2069}"),
        "{header:?}"
    );
    assert!(
        !header.iter().any(|text| text.contains("selected items")
            || text.contains("name conflicts")
            || text.contains("unsupported")),
        "no counts of zero problems: {header:?}"
    );
    editor.click("Times", AccessibleRole::Button);
    // Typing selects the row and completes its other fields from the current time.
    editor.type_into("Modified date", "2025-12-24");
    assert_eq!(editor.value("Modified time"), "03:04:05");
    assert_eq!(editor.value("Modified UTC offset"), "+01:00");
    editor.type_into("Modified time", "18:30");
    // Up changes the part under the cursor: the minutes at the end of the text.
    editor.key(Key::UpArrow);
    assert_eq!(editor.value("Modified time"), "18:31:00");
    editor.type_into("Modified UTC offset", "-05:00");
    let patch = editor.review_and_apply();
    // 2025-12-24 18:31:00 -05:00
    assert_eq!(patch.modified, at(1_766_619_060));
    assert_eq!(patch.created, TimeEdit::Keep);
    assert_eq!(patch.accessed, TimeEdit::Keep);
}

#[test]
fn calendar_and_time_picker_start_from_the_files_time_and_keep_the_dialog_open() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    editor.click("Change Modified", AccessibleRole::Checkbox);
    assert_eq!(editor.value("Modified date"), "2026-01-02");
    assert_eq!(editor.value("Modified time"), "03:04:05");
    render::snapshot(&editor.test, "metadata-times-filled");
    editor.click("Modified: choose date", AccessibleRole::Button);
    render::snapshot(&editor.test, "metadata-times-calendar");
    assert!(editor.texts().iter().any(|text| text == "January 2026"));
    editor.key(Key::RightArrow);
    editor.key(Key::Return);
    assert_eq!(editor.value("Modified date"), "2026-01-03");
    // Escape closes only the calendar, not the whole editor.
    editor.click("Modified: choose date", AccessibleRole::Button);
    editor.key(Key::PageDown);
    editor.key(Key::Escape);
    assert_ne!(editor.test.window.get_metadata_token(), "");
    assert_eq!(editor.value("Modified date"), "2026-01-03");
    editor.click("Modified: choose time", AccessibleRole::Button);
    render::snapshot(&editor.test, "metadata-times-clock");
    editor.key(Key::RightArrow);
    editor.key(Key::UpArrow);
    editor.key(Key::Return);
    assert_eq!(editor.value("Modified time"), "03:05:05");
    let patch = editor.review_and_apply();
    // 2026-01-03 03:05:05 +01:00
    assert_eq!(patch.modified, at(1_767_405_905));
}

#[test]
fn mouse_clicks_inside_the_pickers_keep_them_open() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    editor.click("Change Modified", AccessibleRole::Checkbox);
    editor.click("Modified: choose date", AccessibleRole::Button);
    editor.click("Next month", AccessibleRole::Button);
    assert!(
        editor.texts().iter().any(|text| text == "February 2026"),
        "the calendar stays open on the next month: {:?}",
        editor.texts()
    );
    editor.click("Next month", AccessibleRole::Button);
    assert!(editor.texts().iter().any(|text| text == "March 2026"));
    editor.click("2026-03-14", AccessibleRole::Button);
    assert!(
        ui::try_popup_item(&editor.test.window, "Next month", AccessibleRole::Button).is_none(),
        "choosing a day closes the calendar"
    );
    assert_eq!(editor.value("Modified date"), "2026-03-14");
    editor.click("Modified: choose time", AccessibleRole::Button);
    editor.click("Increase Hour", AccessibleRole::Button);
    editor.click("Increase Hour", AccessibleRole::Button);
    assert!(editor.picker_open(), "the arrows keep the time picker open");
    // A click outside closes the picker and keeps the time it shows.
    editor.click_editor_text("\u{2066}report.txt\u{2069}");
    assert!(!editor.picker_open());
    assert_eq!(editor.value("Modified time"), "05:04:05");
    let patch = editor.review_and_apply();
    // 2026-03-14 05:04:05 +01:00
    assert_eq!(patch.modified, at(1_773_461_045));
}

#[test]
fn the_change_box_shows_whether_the_row_will_be_applied() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    editor.click("Change Modified", AccessibleRole::Checkbox);
    assert_eq!(editor.checked("Change Modified"), "true");
    editor.click("Change Modified", AccessibleRole::Checkbox);
    assert_eq!(editor.checked("Change Modified"), "false");
    // Typing into the unticked row selects it again, and the box says so.
    editor.type_into("Modified date", "2025-12-24");
    assert_eq!(editor.checked("Change Modified"), "true");
    editor.click("Change Modified", AccessibleRole::Checkbox);
    assert_eq!(editor.checked("Change Modified"), "false");
    editor.click("Modified: Now", AccessibleRole::Button);
    assert_eq!(
        editor.checked("Change Modified"),
        "true",
        "Now ticks it too"
    );
    assert!(matches!(
        editor.review_and_apply().modified,
        TimeEdit::Set(_)
    ));
}

/// The instant that a row's three fields show, to the second.
fn shown(editor: &Editor, label: &str) -> SystemTime {
    let text = format!(
        "{} {} {}",
        editor.value(&format!("{label} date")),
        editor.value(&format!("{label} time")),
        editor.value(&format!("{label} UTC offset"))
    );
    let parsed = chrono::DateTime::parse_from_str(&text, "%Y-%m-%d %H:%M:%S %:z")
        .unwrap_or_else(|error| panic!("{label} shows {text}: {error}"));
    UNIX_EPOCH + Duration::from_secs(u64::try_from(parsed.timestamp()).unwrap())
}

#[test]
fn now_shows_the_current_local_time_in_its_row_and_apply_sets_that_moment() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    // The mouse on Modified, the keyboard on Accessed and a screen reader on Created.
    let mut moments = Vec::new();
    let before = SystemTime::now();
    editor.click("Modified: Now", AccessibleRole::Button);
    moments.push(("Modified", before, SystemTime::now()));
    editor.click("Accessed UTC offset", AccessibleRole::TextInput);
    editor.key(Key::Tab);
    let before = SystemTime::now();
    editor.key(Key::Space);
    moments.push(("Accessed", before, SystemTime::now()));
    let before = SystemTime::now();
    editor.act("Created: Now", AccessibleRole::Button);
    moments.push(("Created", before, SystemTime::now()));
    let mut seconds = Vec::new();
    for (label, before, after) in &moments {
        assert_eq!(
            editor.checked(&format!("Change {label}")),
            "true",
            "{label}"
        );
        // The worker's local offset in the fixture is +01:00.
        assert_eq!(editor.value(&format!("{label} UTC offset")), "+01:00");
        let second = shown(&editor, label);
        assert!(
            second + Duration::from_secs(1) > *before && second <= *after,
            "{label} shows the moment it was pressed: {second:?}"
        );
        seconds.push(second);
    }
    // Review and Apply send exactly those moments, sub-seconds included.
    let patch = editor.review_and_apply();
    for ((label, before, after), (edit, second)) in moments.iter().zip([
        (patch.modified, seconds[0]),
        (patch.accessed, seconds[1]),
        (patch.created, seconds[2]),
    ]) {
        let TimeEdit::Set(applied) = edit else {
            panic!("{label} applies {edit:?}");
        };
        assert!(
            applied >= *before && applied <= *after,
            "{label}: the moment Now was pressed"
        );
        assert!(
            applied >= second && applied < second + Duration::from_secs(1),
            "{label}: the second its fields show"
        );
    }
}

#[test]
fn the_first_picker_click_on_an_unticked_row_steps_its_shown_time() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    assert_eq!(
        editor.value("Modified time"),
        "03:04:05",
        "an unticked row shows the file's time"
    );
    assert_eq!(editor.checked("Change Modified"), "false");
    editor.click("Modified: choose time", AccessibleRole::Button);
    editor.click("Increase Hour", AccessibleRole::Button);
    editor.key(Key::Return);
    assert_eq!(editor.value("Modified time"), "04:04:05");
    assert_eq!(editor.checked("Change Modified"), "true");
}

#[test]
fn the_calendar_answers_screen_reader_actions_and_names_its_cursor() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    editor.click("Change Modified", AccessibleRole::Checkbox);
    editor.click("Modified: choose date", AccessibleRole::Button);
    let cursor = |editor: &Editor| {
        editor.state(
            "Modified calendar date",
            AccessibleRole::Spinbox,
            AccessibleStringProperty::Value,
        )
    };
    assert_eq!(cursor(&editor), "2026-01-02");
    editor.key(Key::RightArrow);
    assert_eq!(cursor(&editor), "2026-01-03");
    editor.act("Next month", AccessibleRole::Button);
    assert!(editor.texts().iter().any(|text| text == "February 2026"));
    assert_eq!(cursor(&editor), "2026-02-03");
    editor.act("2026-02-10", AccessibleRole::Button);
    assert_eq!(editor.value("Modified date"), "2026-02-10");
}

#[test]
fn a_ticked_but_unedited_time_keeps_its_sub_seconds() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    editor.click("Change Modified", AccessibleRole::Checkbox);
    assert_eq!(editor.value("Modified time"), "03:04:05");
    // 2026-01-02 03:04:05.789 +01:00, exactly as the worker captured it.
    assert_eq!(
        editor.review_and_apply().modified,
        TimeEdit::Set(UNIX_EPOCH + Duration::from_millis(1_767_319_445_789))
    );
}

#[test]
fn review_notes_an_offset_that_is_not_local_time_on_the_new_date() {
    let mut editor = Editor::open();
    editor.click("Times", AccessibleRole::Button);
    // The file's January time has the winter offset; only its date moves into summer.
    editor.type_into("Modified date", "2026-07-15");
    let (revision, patch) = editor.review();
    // 2026-07-15 03:04:05 +01:00
    let set = UNIX_EPOCH + Duration::from_secs(1_784_081_045);
    assert_eq!(patch.modified, TimeEdit::Set(set));
    let summer = LocalCalendarTime {
        offset_seconds: 7200,
        ..calendar(2026, 7, 15, 4, 4, 5)
    };
    editor.preview_with(revision, patch, HashMap::from([(set, summer)]));
    let review = editor.test.window.get_metadata_preview_text();
    assert!(
        review.contains(
            "Modified uses UTC offset +01:00, but local time on 2026-07-15 is UTC+02:00: \
             the file will show 04:04:05 local time."
        ),
        "{review}"
    );
}

#[test]
fn a_date_typed_in_the_system_order_keeps_that_order_through_arrows_and_the_calendar() {
    let mut editor = Editor::open_with(Some(format("%d.%m.%Y %H:%M:%S", ["", ""])));
    editor.click("Times", AccessibleRole::Button);
    let hint = editor.description("Modified date");
    assert!(
        hint.contains("2026-11-24") && hint.contains("24.11.2026"),
        "{hint}"
    );
    editor.type_into("Modified date", "24.12.2025");
    assert_eq!(editor.value("Modified time"), "03:04:05");
    // The cursor is at the end of the text, which in this order is the year.
    editor.key(Key::UpArrow);
    assert_eq!(editor.value("Modified date"), "24.12.2026");
    editor.click("Modified: choose date", AccessibleRole::Button);
    assert!(editor.texts().iter().any(|text| text == "December 2026"));
    editor.key(Key::RightArrow);
    editor.key(Key::Return);
    assert_eq!(editor.value("Modified date"), "25.12.2026");
    let patch = editor.review_and_apply();
    // 2026-12-25 03:04:05 +01:00
    assert_eq!(patch.modified, at(1_798_164_245));
}

#[test]
fn a_time_typed_on_the_system_12_hour_clock_keeps_it_through_arrows_and_the_picker() {
    let mut editor = Editor::open_with(Some(format("%m/%d/%Y %I:%M:%S %p", ["AM", "PM"])));
    editor.click("Times", AccessibleRole::Button);
    let hint = editor.description("Modified time");
    assert!(
        hint.contains("14:30:00") && hint.contains("02:30:00 PM"),
        "{hint}"
    );
    editor.type_into("Modified time", "2:30 pm");
    assert_eq!(editor.value("Modified date"), "2026-01-02");
    // The cursor is right after the marker, so Up switches it.
    editor.key(Key::UpArrow);
    assert_eq!(editor.value("Modified time"), "02:30:00 AM");
    editor.click("Modified: choose time", AccessibleRole::Button);
    render::snapshot(&editor.test, "metadata-times-clock-12h");
    let shown = editor.texts();
    assert!(
        shown.iter().any(|text| text == "AM") && !shown.iter().any(|text| text == "PM"),
        "{shown:?}"
    );
    // The fourth column switches AM and PM; then the hour column steps the hour.
    for key in [
        Key::RightArrow,
        Key::RightArrow,
        Key::RightArrow,
        Key::UpArrow,
    ] {
        editor.key(key);
    }
    assert!(editor.texts().iter().any(|text| text == "PM"));
    for key in [
        Key::LeftArrow,
        Key::LeftArrow,
        Key::LeftArrow,
        Key::UpArrow,
        Key::Return,
    ] {
        editor.key(key);
    }
    assert_eq!(editor.value("Modified time"), "03:30:00 PM");
    let patch = editor.review_and_apply();
    // 2026-01-02 15:30:00 +01:00
    assert_eq!(patch.modified, at(1_767_364_200));
}
