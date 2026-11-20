//! The Times page with real presenter callbacks: typed dates and times, arrow-key
//! steps and both pickers reach the exact reviewed patch, and the header is plain.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::Presenter;
use slint::{ComponentHandle, platform::Key};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

struct Editor {
    test: ui::TestUi,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    id: OperationId,
    root: PathBuf,
    /// The worker-captured system date and time edit format, if any.
    format: Option<DateTimeInputFormat>,
}

fn calendar(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> LocalCalendarTime {
    LocalCalendarTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        offset_seconds: 3600,
    }
}

impl Editor {
    fn open() -> Self {
        Self::open_with(None)
    }

    fn open_with(format: Option<DateTimeInputFormat>) -> Self {
        let test = ui::TestUi::new();
        let root = std::env::temp_dir();
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        midnight_ui::install(
            &test.window,
            state.clone(),
            Rc::new({
                let commands = commands.clone();
                move |command| {
                    commands.borrow_mut().push(command);
                    true
                }
            }),
        );
        let navigation = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(1),
            root.join("report.txt"),
            EntryKind::File,
        )]
        .into();
        state.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        midnight_ui::refresh(&test.window, &state.borrow());
        let mut editor = Self {
            test,
            state,
            commands,
            id: OperationId(0),
            root,
            format,
        };
        editor.test.draw();
        editor.test.window.invoke_row_click(0, 1, false, false);
        editor.test.window.invoke_open_metadata(0, false);
        editor.id = editor
            .commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::PrepareOperation(request)
                    if request.kind == MutationKind::EditMetadata =>
                {
                    Some(request.id)
                }
                _ => None,
            })
            .unwrap();
        editor.preview(0, MetadataPatch::default());
        editor
    }

    fn preview(&mut self, revision: u64, patch: MetadataPatch) {
        let preview = MetadataPreview {
            id: self.id,
            revision,
            patch,
            localized_times: Default::default(),
            time_input_format: self.format.clone(),
            time_defaults: MetadataTimeDefaults {
                now: Some(calendar(2026, 11, 19, 10, 0, 0)),
                modified: Some(calendar(2026, 1, 2, 3, 4, 5)),
                ..Default::default()
            },
            capabilities: MetadataCapabilities {
                created: true,
                modified: true,
                accessed: true,
                ..Default::default()
            },
            total: 1,
            conflicts: 0,
            unsupported: 0,
            hard_linked: 0,
            sample: vec![MetadataPreviewRow {
                path: self.root.join("report.txt"),
                proposed_name: None,
                current: MetadataSnapshot::default(),
                proposed: MetadataSnapshot::default(),
                issue: None,
            }],
            errors: vec![],
        };
        self.state
            .borrow_mut()
            .apply(Event::Operation(OperationEvent::MetadataPreview(Arc::new(
                preview,
            ))));
        midnight_ui::refresh(&self.test.window, &self.state.borrow());
        self.test.draw();
    }

    fn click(&mut self, label: &str, role: AccessibleRole) {
        ui::click_popup(&self.test.window, label, role);
        self.test.draw();
    }

    fn key(&mut self, key: Key) {
        ui::key(&self.test.window, key);
        self.test.draw();
    }

    fn type_into(&mut self, label: &str, text: &str) {
        self.click(label, AccessibleRole::TextInput);
        ui::chord(&self.test.window, &[Key::Control], "a");
        for character in text.chars() {
            ui::key(&self.test.window, character.to_string());
        }
        self.test.draw();
    }

    fn value(&self, label: &str) -> String {
        ui::popup_item(&self.test.window, label, AccessibleRole::TextInput)
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap_or_default()
            .to_string()
    }

    fn description(&self, label: &str) -> String {
        ui::popup_item(&self.test.window, label, AccessibleRole::TextInput)
            .accessible_string_property(AccessibleStringProperty::Description)
            .unwrap_or_default()
            .to_string()
    }

    fn texts(&self) -> Vec<String> {
        let popups = WindowInner::from_pub(self.test.window.window()).active_popups();
        ui::text_values(ItemRc::new_root(popups.last().unwrap().component.clone()))
            .into_iter()
            .map(|text| text.to_string())
            .collect()
    }

    /// Review, answer with the worker's preview of exactly that patch, then Apply.
    fn review_and_apply(&mut self) -> MetadataPatch {
        self.click("Review changes", AccessibleRole::Button);
        let (revision, patch) = self
            .commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PreviewMetadata {
                    revision, patch, ..
                } => Some((*revision, patch.clone())),
                _ => None,
            })
            .expect("Review sends the typed draft");
        self.preview(revision, patch.clone());
        self.click("Apply", AccessibleRole::Button);
        let id = self.id;
        assert!(
            self.commands
                .borrow()
                .iter()
                .any(|command| matches!(command,
            Command::ConfirmMetadata { id: confirmed, revision: reviewed, .. }
            if *confirmed == id && *reviewed == revision))
        );
        patch
    }
}

fn format(pattern: &str, am_pm: [&str; 2]) -> DateTimeInputFormat {
    DateTimeInputFormat {
        pattern: pattern.into(),
        am_pm: am_pm.map(String::from),
    }
}

fn at(seconds: u64) -> TimeEdit {
    TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(seconds))
}

#[test]
fn typed_date_time_and_offset_with_an_arrow_step_apply_the_exact_instant() {
    let mut editor = Editor::open();
    let header = editor.texts();
    assert!(header.iter().any(|text| text == "report.txt"), "{header:?}");
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
