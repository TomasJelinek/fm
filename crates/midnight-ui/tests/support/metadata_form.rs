//! The Attributes dialog on one file with real presenter callbacks, as the
//! `metadata_times` and `metadata_editor` tests drive it: the worker's preview, clicks,
//! keys, the wheel, field values, the keyboard focus and the reviewed patch.
#![allow(dead_code)]
use crate::support::ui;
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::ItemRc,
    items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::Presenter;
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct Editor {
    pub test: ui::TestUi,
    pub state: Rc<RefCell<Presenter>>,
    pub commands: Rc<RefCell<Vec<Command>>>,
    pub id: OperationId,
    pub root: PathBuf,
    /// The worker-captured system date and time edit format, if any.
    pub format: Option<DateTimeInputFormat>,
    /// The worker's capabilities: which times the file system can set.
    pub capabilities: MetadataCapabilities,
    /// The starting values the worker's previews carry.
    pub defaults: MetadataTimeDefaults,
    /// Slint's clock when the editor was opened with a virtual one.
    pub clock: Option<Rc<Cell<Duration>>>,
}

pub fn calendar(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> LocalCalendarTime {
    LocalCalendarTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanosecond: 0,
        offset_seconds: 3600,
    }
}

/// Created, modified and accessed can all be set.
pub fn all_times() -> MetadataCapabilities {
    MetadataCapabilities {
        created: true,
        modified: true,
        accessed: true,
        ..Default::default()
    }
}

impl Editor {
    pub fn open() -> Self {
        Self::start(ui::TestUi::new(), None, None, all_times())
    }

    pub fn open_with(format: Option<DateTimeInputFormat>) -> Self {
        Self::start(ui::TestUi::new(), None, format, all_times())
    }

    /// An editor whose Slint timers only advance with [`Self::advance`].
    pub fn open_with_clock() -> Self {
        let (test, clock) = ui::TestUi::with_virtual_clock();
        Self::start(test, Some(clock), None, all_times())
    }

    /// An editor on a file system that can set only the times in `capabilities`.
    pub fn open_capable(capabilities: MetadataCapabilities) -> Self {
        Self::start(ui::TestUi::new(), None, None, capabilities)
    }

    fn start(
        test: ui::TestUi,
        clock: Option<Rc<Cell<Duration>>>,
        format: Option<DateTimeInputFormat>,
        capabilities: MetadataCapabilities,
    ) -> Self {
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
            capabilities,
            defaults: MetadataTimeDefaults {
                now: Some(calendar(2026, 11, 19, 10, 0, 0)),
                // The file's time has sub-seconds that the fields do not show.
                modified: Some(LocalCalendarTime {
                    nanosecond: 789_000_000,
                    ..calendar(2026, 1, 2, 3, 4, 5)
                }),
                ..Default::default()
            },
            clock,
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

    pub fn preview(&mut self, revision: u64, patch: MetadataPatch) {
        self.preview_with(revision, patch, HashMap::new());
    }

    /// The worker's preview, with the native-zone calendars of the patch's set times.
    pub fn preview_with(
        &mut self,
        revision: u64,
        patch: MetadataPatch,
        patch_calendars: HashMap<SystemTime, LocalCalendarTime>,
    ) {
        let preview = MetadataPreview {
            id: self.id,
            revision,
            patch,
            localized_times: Default::default(),
            patch_calendars,
            time_input_format: self.format.clone(),
            time_defaults: self.defaults,
            capabilities: self.capabilities,
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

    /// Advance the virtual clock of [`Self::open_with_clock`] and draw.
    pub fn advance(&mut self, by: Duration) {
        let clock = self.clock.as_ref().expect("an editor with a virtual clock");
        ui::advance(clock, by);
        self.test.draw();
    }

    /// A control of the open picker, else of the editor under it.
    pub fn item(&self, label: &str, role: AccessibleRole) -> ItemRc {
        self.try_item(label, role)
            .unwrap_or_else(|| panic!("no accessible control: {label}"))
    }

    pub fn try_item(&self, label: &str, role: AccessibleRole) -> Option<ItemRc> {
        let inner = WindowInner::from_pub(self.test.window.window());
        let popups = inner.active_popups();
        popups
            .iter()
            .rev()
            .filter(|popup| !matches!(popup.window_kind, i_slint_core::window::WindowKind::ToolTip))
            .find_map(|popup| find(ItemRc::new_root(popup.component.clone()), label, role))
    }

    /// The window position of the centre of a topmost-popup control.
    pub fn centre(&self, label: &str, role: AccessibleRole) -> slint::LogicalPosition {
        let item = self.item(label, role);
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        )
    }

    /// A pointer click on the centre of a control: on whatever is drawn there.
    pub fn click(&mut self, label: &str, role: AccessibleRole) {
        let position = self.centre(label, role);
        ui::click(&self.test.window, position.x, position.y);
        self.test.draw();
    }

    /// Click a text of the editor itself, under any picker: with a picker open, that
    /// click only closes the picker.
    pub fn click_editor_text(&mut self, text: &str) {
        let position = {
            let inner = WindowInner::from_pub(self.test.window.window());
            let popups = inner.active_popups();
            let editor = popups
                .iter()
                .find(|popup| {
                    !matches!(popup.window_kind, i_slint_core::window::WindowKind::ToolTip)
                })
                .expect("the editor");
            let item = find(
                ItemRc::new_root(editor.component.clone()),
                text,
                AccessibleRole::Text,
            )
            .unwrap_or_else(|| panic!("the editor shows no {text:?}"));
            let rect = item.geometry();
            let origin = item.map_to_native_window(rect.origin);
            slint::LogicalPosition::new(origin.x + 4.0, origin.y + rect.size.height / 2.0)
        };
        ui::click(&self.test.window, position.x, position.y);
        self.test.draw();
    }

    /// Press the left button on a control and keep it down.
    pub fn press(&mut self, label: &str, role: AccessibleRole) {
        let position = self.centre(label, role);
        ui::pointer_button(&self.test.window, position, PointerEventButton::Left, true);
        self.test.draw();
    }

    /// Release the left button where `position` is.
    pub fn release(&mut self, position: slint::LogicalPosition) {
        ui::pointer_button(&self.test.window, position, PointerEventButton::Left, false);
        self.test.draw();
    }

    /// Move the pointer, a button held or not.
    pub fn move_to(&mut self, position: slint::LogicalPosition) {
        self.test
            .window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.test.draw();
    }

    /// Turn the wheel over a control: positive `delta_y` is up, 60 px a notch.
    pub fn wheel(&mut self, label: &str, role: AccessibleRole, delta_y: f32) {
        let position = self.centre(label, role);
        self.move_to(position);
        self.test
            .window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y,
            });
        self.test.draw();
    }

    pub fn key(&mut self, key: impl Into<slint::SharedString>) {
        ui::key(&self.test.window, key);
        self.test.draw();
    }

    pub fn chord(&mut self, modifiers: &[Key], key: impl Into<slint::SharedString>) {
        ui::chord(&self.test.window, modifiers, key);
        self.test.draw();
    }

    pub fn type_into(&mut self, label: &str, text: &str) {
        self.click(label, AccessibleRole::TextInput);
        ui::chord(&self.test.window, &[Key::Control], "a");
        for character in text.chars() {
            ui::key(&self.test.window, character.to_string());
        }
        self.test.draw();
    }

    pub fn value(&self, label: &str) -> String {
        self.state(
            label,
            AccessibleRole::TextInput,
            AccessibleStringProperty::Value,
        )
    }

    pub fn description(&self, label: &str) -> String {
        self.state(
            label,
            AccessibleRole::TextInput,
            AccessibleStringProperty::Description,
        )
    }

    /// The texts the topmost popup draws; hidden measuring texts are left out.
    pub fn texts(&self) -> Vec<String> {
        let popups = WindowInner::from_pub(self.test.window.window()).active_popups();
        ui::visible_text_values(ItemRc::new_root(popups.last().unwrap().component.clone()))
            .into_iter()
            .map(|text| text.to_string())
            .collect()
    }

    /// The accessible state of a checkbox or spin box in the topmost popup.
    pub fn state(
        &self,
        label: &str,
        role: AccessibleRole,
        what: AccessibleStringProperty,
    ) -> String {
        self.item(label, role)
            .accessible_string_property(what)
            .unwrap_or_default()
            .to_string()
    }

    pub fn checked(&self, label: &str) -> String {
        self.state(
            label,
            AccessibleRole::Checkbox,
            AccessibleStringProperty::Checked,
        )
    }

    /// The accessible label and value of the control with the keyboard focus: the
    /// focused item's own, else those of the nearest labelled item around it (a button's
    /// focus scope sits inside the labelled button).
    pub fn focused(&self) -> (String, String) {
        use i_slint_core::item_tree::ParentItemTraversalMode;
        let inner = WindowInner::from_pub(self.test.window.window());
        let mut item = inner.focus_item.borrow().upgrade();
        while let Some(current) = item {
            let label = current
                .accessible_string_property(AccessibleStringProperty::Label)
                .unwrap_or_default();
            if !label.is_empty() {
                let value = current
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap_or_default();
                return (label.to_string(), value.to_string());
            }
            item = current.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        Default::default()
    }

    /// Whether the open time picker or calendar shows (a popup over the editor).
    pub fn picker_open(&self) -> bool {
        WindowInner::from_pub(self.test.window.window())
            .active_popups()
            .iter()
            .filter(|popup| !matches!(popup.window_kind, i_slint_core::window::WindowKind::ToolTip))
            .count()
            > 1
    }

    /// Run an accessibility action, as a screen reader does, on a topmost-popup control.
    pub fn act(&mut self, label: &str, role: AccessibleRole) {
        self.item(label, role)
            .accessible_action(&AccessibilityAction::Default);
        self.test.draw();
    }

    /// Review: the revision and patch that the draft sent to the worker.
    pub fn review(&mut self) -> (u64, MetadataPatch) {
        self.click("Review changes", AccessibleRole::Button);
        self.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PreviewMetadata {
                    revision, patch, ..
                } => Some((*revision, patch.clone())),
                _ => None,
            })
            .expect("Review sends the typed draft")
    }

    /// Review, answer with the worker's preview of exactly that patch, then Apply.
    pub fn review_and_apply(&mut self) -> MetadataPatch {
        let (revision, patch) = self.review();
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

    /// Review, then go back to editing with the worker's answer shown.
    pub fn review_and_return(&mut self) -> MetadataPatch {
        let (revision, patch) = self.review();
        self.preview(revision, patch.clone());
        self.click("Back to editing", AccessibleRole::Button);
        self.click("Times", AccessibleRole::Button);
        patch
    }
}

/// The control with this accessible label and role at or below `item`.
pub fn find(item: ItemRc, label: &str, role: AccessibleRole) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, label, role) {
            return Some(found);
        }
    }
    None
}

pub fn format(pattern: &str, am_pm: [&str; 2]) -> DateTimeInputFormat {
    DateTimeInputFormat {
        pattern: pattern.into(),
        am_pm: am_pm.map(String::from),
    }
}

pub fn at(seconds: u64) -> TimeEdit {
    TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(seconds))
}
