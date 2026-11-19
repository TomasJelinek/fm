//! Actual Options callbacks preserve drafts and reveal revision-bound comparator errors.
//! NOT RUN until the coordinated UI lease.
#[path = "support/popup_scroll.rs"]
mod popup_scroll;
#[cfg(unix)]
#[path = "support/unix_raw_paths.rs"]
mod raw_paths;
#[cfg(windows)]
#[path = "support/windows_raw_paths.rs"]
mod raw_paths;
#[path = "support/render.rs"]
mod render;
use crate::support::ui;

use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{ComparatorOptionsView, Presenter, install, refresh};
use slint::{ComponentHandle, Model, platform::Key};
use std::{
    cell::{Cell, RefCell},
    io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
}
impl Harness {
    fn new(width: usize, height: usize, font: u16) -> Self {
        Self::with_clock(width, height, font, None)
    }
    fn with_clock(
        width: usize,
        height: usize,
        font: u16,
        clock: Option<Rc<Cell<Duration>>>,
    ) -> Self {
        Self::with_accepted(
            width,
            height,
            clock,
            AcceptedPreferences {
                preferences: UserPreferences {
                    desktop_font_size: font,
                    touch_font_size: font,
                    ..Default::default()
                },
                comparators: FileComparatorSnapshot::default(),
            },
        )
    }
    fn with_accepted(
        width: usize,
        height: usize,
        clock: Option<Rc<Cell<Duration>>>,
        accepted: AcceptedPreferences,
    ) -> Self {
        let mut ui = ui::TestUi::with_clock(clock);
        ui.resize(width, height);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).to_owned();
        let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let captured = sent.clone();
        install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                captured.borrow_mut().push(command);
                true
            }),
        );
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(accepted))));
        refresh(&ui.window, &presenter.borrow());
        ui.window.invoke_show_options();
        ui.window.set_options_topic_index(10);
        let mut harness = Self {
            ui,
            presenter,
            sent,
        };
        harness.settle();
        harness
    }
    fn settle(&mut self) {
        for _ in 0..3 {
            self.ui.draw();
        }
    }
    fn saves(&self) -> Vec<(PreferencesRevision, UserPreferences)> {
        self.sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::SavePreferences {
                    revision,
                    preferences,
                } => Some((*revision, *preferences.clone())),
                _ => None,
            })
            .collect()
    }
    fn event(&mut self, event: Event) {
        let sent = self.sent.clone();
        self.presenter
            .borrow_mut()
            .apply_with_sender(event, &|command| {
                sent.borrow_mut().push(command);
                true
            });
        refresh(&self.ui.window, &self.presenter.borrow());
        self.settle();
    }
    fn ok_from_general(&mut self) {
        self.ui.window.set_options_topic_index(0);
        self.settle();
        ui::click_popup(&self.ui.window, "OK", AccessibleRole::Button);
        self.settle();
    }
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let geometry = item.geometry();
    let origin = item.map_to_native_window(geometry.origin);
    (
        origin.x,
        origin.y,
        origin.x + geometry.width(),
        origin.y + geometry.height(),
    )
}

fn instantiated_list_rows(item: ItemRc) -> Vec<String> {
    let mut rows = Vec::new();
    if item.accessible_role() == AccessibleRole::ListItem {
        rows.push(format!(
            "index={:?}, selected={:?}, label={:?}, bounds={:?}",
            item.accessible_string_property(AccessibleStringProperty::ItemIndex),
            item.accessible_string_property(AccessibleStringProperty::ItemSelected),
            item.accessible_string_property(AccessibleStringProperty::Label),
            bounds(&item)
        ));
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        rows.extend(instantiated_list_rows(item));
    }
    rows
}

fn focused_and_visible(test: &ui::TestUi, label: &str) {
    let item = ui::popup_item(&test.window, label, AccessibleRole::TextInput);
    let area = bounds(&item);
    assert!(
        area.0 >= 0.0
            && area.1 >= 0.0
            && area.2 <= test.width as f32 + 1.0
            && area.3 <= test.height as f32 + 1.0,
        "{label}: {area:?}"
    );
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<Flickable>().is_some() {
            let clip = bounds(&ancestor);
            assert!(
                area.0 >= clip.0 - 1.0
                    && area.1 >= clip.1 - 1.0
                    && area.2 <= clip.2 + 1.0
                    && area.3 <= clip.3 + 1.0,
                "{label}: {area:?} outside {clip:?}"
            );
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    let inner = WindowInner::from_pub(test.window.window());
    let mut focused = inner.focus_item.borrow().upgrade();
    while let Some(current) = focused {
        if current == item {
            return;
        }
        focused = current.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("{label} was revealed without input focus");
}

#[test]
fn other_page_ok_reveals_invalid_disabled_external_arguments_at_compact_large_font() {
    let mut harness = Harness::new(320, 200, 32);
    let window = harness.ui.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.invoke_action(0);
    let mut form = view.get_draft();
    form.enabled = false;
    form.target = 3;
    form.executable = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("comparator.exe")
        .to_string_lossy()
        .into_owned()
        .into();
    view.invoke_edit(form);
    view.invoke_argument(2, 1, "".into()); // Remove complete {right}; disabled rules still validate.
    harness.ok_from_general();
    assert!(harness.saves().is_empty());
    assert_eq!(window.get_options_topic_index(), 10);
    assert!(window.get_options_open());
    assert_eq!(view.get_error_field(), 4);
    assert!(view.get_dirty());
    assert!(!view.get_draft().enabled);
    focused_and_visible(&harness.ui, "Comparator argument 1");
}

#[test]
fn backend_regex_feedback_reveals_submitted_row_and_pattern_from_another_page() {
    let mut harness = Harness::new(1100, 740, 32);
    let window = harness.ui.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.invoke_action(0);
    let mut form = view.get_draft();
    form.mode = 1;
    form.expression = "(".into();
    form.enabled = false;
    view.invoke_edit(form);
    view.invoke_action(0);
    view.invoke_action(2); // Invalid disabled row is now second in the submitted order.
    harness.ok_from_general();
    let saves = harness.saves();
    assert_eq!(saves.len(), 1);
    let diagnostic = FileComparatorDiagnostic {
        rule: Some(1),
        field: FileComparatorField::Expression,
        message: "Unclosed group".into(),
    };
    harness.event(Event::PreferencesSaved {
        revision: saves[0].0,
        result: Err(diagnostic.into()),
    });
    assert_eq!(window.get_options_topic_index(), 10);
    assert!(window.get_options_open());
    assert_eq!(view.get_selected(), 1);
    assert_eq!(view.get_draft().expression, "(");
    assert!(!view.get_draft().enabled);
    focused_and_visible(&harness.ui, "Comparator filename pattern");
}

#[test]
fn persistence_failure_retains_ui_draft_and_cancel_restores_accepted_launch_snapshot() {
    let mut harness = Harness::new(1100, 740, 14);
    let window = harness.ui.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    let accepted = FileComparatorSnapshot::default();
    view.invoke_action(0);
    let mut form = view.get_draft();
    form.target = 1;
    form.expression = "*.log".into();
    view.invoke_edit(form);
    harness.ok_from_general();
    let saves = harness.saves();
    assert_eq!(saves.len(), 1);
    harness.event(Event::PreferencesSaved {
        revision: saves[0].0,
        result: Err(FsError::from_io(
            "save",
            Path::new("preferences"),
            io::Error::other("not saved"),
        )
        .into()),
    });
    assert!(window.get_options_open());
    assert!(view.get_dirty());
    assert_eq!(view.get_rule_count(), 1);
    assert_eq!(view.get_draft().expression, "*.log");
    let launch = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(Arc::ptr_eq(&launch.comparators, &accepted.preferences));
    assert_eq!(launch.comparator_policy, accepted.id);
    ui::click_popup(&window, "Cancel", AccessibleRole::Button);
    harness.settle();
    assert!(!window.get_options_open());
    let launch = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(launch.comparators.rules.is_empty());
    assert!(Arc::ptr_eq(&launch.comparators, &accepted.preferences));
}

#[test]
fn leaving_picker_page_releases_busy_controls_even_before_a_picker_reply() {
    let clock = Rc::new(Cell::new(Duration::ZERO));
    let mut harness = Harness::with_clock(1100, 740, 14, Some(clock.clone()));
    let window = harness.ui.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.invoke_action(0);
    let mut form = view.get_draft();
    form.target = 3;
    form.executable = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("compare.exe")
        .to_string_lossy()
        .into_owned()
        .into();
    view.invoke_edit(form);
    view.invoke_browse();
    assert!(view.get_busy());
    assert!(
        harness
            .sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::PickPath { .. }))
    );
    window.set_options_topic_index(0);
    harness.settle();
    assert!(
        harness
            .sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelPathPicker { .. }))
    );
    // No PathPicked reply is delivered. The context change that delivered the
    // cancellation also publishes the emptied slot; it does not wait for the open
    // picker's one-second check.
    assert!(!view.get_busy());
    clock.set(Duration::from_millis(30));
    harness.settle();
    window.set_options_topic_index(10);
    harness.settle();
    assert!(!view.get_busy());
}

#[test]
fn comparators_navigation_preserves_older_topic_ids_on_sidebar_and_compact_picker() {
    use i_slint_core::accessibility::AccessibleStringProperty;
    let mut harness = Harness::new(1100, 740, 14);
    harness.ui.options_topic(0);
    ui::click_popup(&harness.ui.window, "Comparators", AccessibleRole::Button);
    harness.settle();
    assert_eq!(harness.ui.window.get_options_topic_index(), 10);
    harness.ui.resize(320, 200);
    harness.ui.options_topic(0);
    for (topic, label) in [
        (5, "Plugins"),
        (10, "Comparators"),
        (9, "Custom Commands"),
        (4, "Editors"),
    ] {
        harness.ui.options_topic(topic);
        let picker = ui::popup_item(
            &harness.ui.window,
            "Options topic",
            AccessibleRole::Combobox,
        );
        assert_eq!(
            picker
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some(label)
        );
    }
}

#[test]
fn stale_picker_error_does_not_hide_a_later_generic_save_failure() {
    // This backend fixture publishes the single '*' rule so its raw Arc/key is
    // coherent. Presentation receives metadata only, as it does in production.
    struct Policy(Arc<FileComparatorPreferences>);
    impl CompiledFileComparatorPolicy for Policy {
        fn preferences(&self) -> &Arc<FileComparatorPreferences> {
            &self.0
        }
        fn select(&self, paths: &[std::path::PathBuf; 2]) -> Option<usize> {
            paths
                .iter()
                .all(|path| path.file_name().and_then(|name| name.to_str()).is_some())
                .then_some(0)
        }
        fn matcher_budget_bytes(&self) -> usize {
            0
        }
    }
    let comparators = FileComparatorPreferences {
        rules: vec![FileComparatorRule {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*".into(),
            case_sensitive: false,
            target: FileComparatorTarget::External(FileComparatorProgram {
                executable: Path::new(env!("CARGO_MANIFEST_DIR")).join("compare.exe"),
                arguments: vec!["{left}".into(), "{right}".into()],
                working_directory: None,
            }),
        }],
    };
    let backend = FileComparatorPolicyStore::default();
    let policy = PreparedFileComparatorPolicy::new(Arc::new(Policy(Arc::new(comparators.clone()))));
    let snapshot = backend.publish(policy.with_id(backend.prepare_id().unwrap()));
    let mut harness = Harness::with_accepted(
        1100,
        740,
        None,
        AcceptedPreferences {
            preferences: UserPreferences {
                file_comparators: comparators,
                ..Default::default()
            },
            comparators: snapshot,
        },
    );
    let window = harness.ui.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.invoke_select(0);
    view.invoke_browse();
    let request = harness
        .sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PickPath { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    harness.event(Event::PathPicked {
        request,
        result: Err(FsError::from_io(
            "pick",
            Path::new("compare.exe"),
            io::Error::other("picker failed"),
        )),
    });
    assert!(!view.get_dirty());
    assert!(!view.get_error().is_empty());
    assert_eq!(view.get_error_field(), -1);
    harness.ok_from_general();
    let saves = harness.saves();
    assert_eq!(saves.len(), 1);
    harness.event(Event::PreferencesSaved {
        revision: saves[0].0,
        result: Err(FsError::from_io(
            "save",
            Path::new("preferences"),
            io::Error::other("disk write failed"),
        )
        .into()),
    });
    let error = ui::popup_item(&window, "Options error", AccessibleRole::Text);
    assert!(error.is_visible());
    let messages = ui::visible_text_values(error);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("disk write failed")),
        "{messages:?}"
    );
    assert!(window.get_options_open());
}

impl Harness {
    fn click(&mut self, label: &str, role: AccessibleRole) {
        if role == AccessibleRole::Button && matches!(label, "OK" | "Cancel") {
            let item = ui::popup_item(&self.ui.window, label, role);
            let geometry = item.geometry();
            let area = bounds(&item);
            assert_eq!(
                self.ui.window.window().size(),
                slint::PhysicalSize::new(self.ui.width as u32, self.ui.height as u32)
            );
            assert!(item.is_visible());
            assert!(geometry.width() > 0.0 && geometry.height() > 0.0);
            assert!(
                area.0 >= 0.0
                    && area.1 >= 0.0
                    && area.2 <= self.ui.width as f32
                    && area.3 <= self.ui.height as f32,
                "{label} must fit the actual window before pointer activation: {area:?}"
            );
            if self.ui.window.get_density_index() == 1 {
                // This workflow's Touch profile is font32: Theme.control-height is 46px.
                assert_eq!(self.ui.window.get_touch_font_size(), 32);
                assert!(
                    geometry.width() >= 44.0 && geometry.height() >= 44.0,
                    "{label} must retain its touch target: {geometry:?}"
                );
            }
        }
        popup_scroll::click(&mut self.ui, label, role);
    }

    fn input(&mut self, label: &str, value: &str) {
        self.click(label, AccessibleRole::TextInput);
        ui::popup_item(&self.ui.window, label, AccessibleRole::TextInput)
            .accessible_action(&AccessibilityAction::SetValue(value.into()));
        self.settle();
    }

    fn choose_comparator_target(&mut self, target: i32) {
        let current = self
            .ui
            .window
            .global::<ComparatorOptionsView>()
            .get_draft()
            .target;
        self.click("Comparator target", AccessibleRole::Combobox);
        for _ in 0..(target - current).abs() {
            ui::key(
                &self.ui.window,
                if target > current {
                    Key::DownArrow
                } else {
                    Key::UpArrow
                },
            );
        }
        ui::key(&self.ui.window, Key::Return);
        self.settle();
        assert_eq!(
            self.ui
                .window
                .global::<ComparatorOptionsView>()
                .get_draft()
                .target,
            target
        );
    }

    fn select_comparator_row(&mut self, row: usize) {
        // The model supplies the accessible row identifier, not the expected draft.
        let window = self.ui.window.clone_strong();
        let view = window.global::<ComparatorOptionsView>();
        let labels = view.get_labels();
        let label = labels.row_data(row).unwrap();
        if ui::try_popup_item(&window, label.as_str(), AccessibleRole::ListItem).is_none() {
            // Compact ListView instantiates only its visible row. Reveal the selected
            // row through the outer form, then wheel the list before finding the target.
            let selected = view.get_selected();
            let selected_label = labels.row_data(usize::try_from(selected).unwrap()).unwrap();
            self.click(selected_label.as_str(), AccessibleRole::ListItem);
            let (area, row_height, clips) = {
                let current =
                    ui::popup_item(&window, selected_label.as_str(), AccessibleRole::ListItem);
                let mut clips = Vec::new();
                let mut parent = current.parent_item(ParentItemTraversalMode::StopAtPopups);
                while let Some(item) = parent {
                    if let Some(flick) = item.downcast::<Flickable>() {
                        clips.push((bounds(&item), flick.as_pin_ref().content_y().get()));
                    }
                    parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
                }
                (bounds(&current), current.geometry().height(), clips)
            };
            let list = ui::popup_item(&window, "Comparator rules", AccessibleRole::List);
            let delta = (selected as f32 - row as f32) * row_height;
            assert!(row_height > 0.0 && delta != 0.0);
            eprintln!(
                "comparator row {selected}->{row}, count={}, list={:?}, clips={clips:?}, rows={:?}",
                labels.row_count(),
                bounds(&list),
                instantiated_list_rows(list.clone())
            );
            let capture = format!(
                "comparator-workflow-{}x{}-row-{selected}-to-{row}",
                self.ui.width, self.ui.height
            );
            render::snapshot(&self.ui, &format!("{capture}-before-wheel"));
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(
                        (area.0 + area.2) / 2.0,
                        (area.1 + area.3) / 2.0,
                    ),
                    delta_x: 0.0,
                    delta_y: delta,
                });
            self.settle();
            render::snapshot(&self.ui, &format!("{capture}-after-wheel"));
            assert_eq!(view.get_selected(), selected, "wheel must not select a row");
            assert!(
                ui::try_popup_item(&window, label.as_str(), AccessibleRole::ListItem).is_some(),
                "one measured list scroll must instantiate {label:?}; rows={:?}",
                instantiated_list_rows(list)
            );
        }
        // Reacquire the delegate after scrolling: ListView may replace its instances.
        self.click(label.as_str(), AccessibleRole::ListItem);
        assert_eq!(view.get_selected(), row as i32);
    }

    fn arguments(&self) -> Vec<String> {
        self.ui
            .window
            .global::<ComparatorOptionsView>()
            .get_argument_rows()
            .iter()
            .map(|value| value.to_string())
            .collect()
    }

    fn latest_picker(&self) -> (DesktopRequestId, PathPickerRequest) {
        self.sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PickPath { request, input } => Some((*request, input.clone())),
                _ => None,
            })
            .expect("Browse must request the native picker")
    }

    fn reopen_comparators(&mut self) {
        self.ui.window.invoke_show_options();
        self.settle();
        self.ui.options_topic(10);
        self.settle();
    }
}

// The backend seam owns this immutable raw Arc. No matcher is copied into the UI test.
struct SavedOptionsPolicy(Arc<FileComparatorPreferences>);
impl CompiledFileComparatorPolicy for SavedOptionsPolicy {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.0
    }

    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        panic!("Options editing must never evaluate comparison filenames")
    }

    fn matcher_budget_bytes(&self) -> usize {
        0
    }
}

fn successful_external_workflow(width: usize, height: usize, font: u16, density: UiDensity) {
    let backend = FileComparatorPolicyStore::default();
    let initial = backend.snapshot();
    let mut harness = Harness::with_accepted(
        width,
        height,
        None,
        AcceptedPreferences {
            preferences: UserPreferences {
                density,
                desktop_font_size: font,
                touch_font_size: font,
                disabled_viewer_plugins: vec!["disabled.viewer.fixture".into()],
                ..Default::default()
            },
            comparators: initial.clone(),
        },
    );
    let window = harness.ui.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    assert_eq!(
        window.window().size(),
        slint::PhysicalSize::new(width as u32, height as u32)
    );
    assert_eq!(
        window.get_density_index(),
        i32::from(density == UiDensity::Touch)
    );
    assert_eq!(window.get_desktop_font_size(), i32::from(font));
    assert_eq!(window.get_touch_font_size(), i32::from(font));
    assert_eq!(view.get_rule_count(), 0);
    assert_eq!(view.get_labels().row_count(), 1);
    assert!(!view.get_dirty());
    let empty = ui::popup_item(&window, "Comparator empty overrides", AccessibleRole::Text);
    assert!(
        ui::text_values(empty)
            .iter()
            .any(|text| text.contains("No overrides"))
    );

    harness.click("Refresh comparator image types", AccessibleRole::Button);
    let disabled_plugins = harness
        .sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::ListComparatorCapabilities { disabled_plugins } => {
                Some(disabled_plugins.clone())
            }
            _ => None,
        })
        .expect("Refresh must request current-session decoder metadata");
    let startup = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(Arc::ptr_eq(&disabled_plugins, &startup.disabled_plugins));
    harness.event(Event::ComparatorCapabilitiesListed(
        ComparatorCapabilities {
            revision: 17,
            disabled_plugins,
            images: Ok(Arc::from([ComparatorImageCapability {
                provider_id: "opaque.provider.742".into(),
                extensions: vec!["qoi".into(), "tga".into()],
            }])),
        },
    ));
    harness.click("Comparator enabled image types", AccessibleRole::Text);
    let images = ui::popup_item(
        &window,
        "Comparator enabled image types",
        AccessibleRole::Text,
    );
    assert!(
        ui::visible_text_values(images)
            .iter()
            .any(|text| text.as_str() == "*.qoi; *.tga → Automatic → Built-in image")
    );
    harness.click("Comparator text fallback", AccessibleRole::Text);
    let text = ui::popup_item(&window, "Comparator text fallback", AccessibleRole::Text);
    assert!(
        ui::visible_text_values(text)
            .iter()
            .any(|text| text.as_str() == "Other regular files → Automatic → Built-in text")
    );
    assert_eq!(view.get_rule_count(), 0);
    assert_eq!(view.get_labels().row_count(), 1);
    assert!(!view.get_dirty());
    assert!(harness.saves().is_empty());
    render::snapshot(
        &harness.ui,
        &format!("comparator-workflow-{width}x{height}-font{font}-coverage"),
    );

    harness.click("Add comparator rule", AccessibleRole::Button);
    assert_eq!(view.get_rule_count(), 1);
    assert_eq!(view.get_selected(), 0);
    harness.input("Comparator filename pattern", "*.c;*.cpp");
    harness.choose_comparator_target(3);

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("comparator workflow 文档 e\u{301}");
    let [native_folder, alias_folder] = raw_paths::same_display(&root);
    let executable = native_folder.join("compare.exe");
    let alias = alias_folder.join("compare.exe");
    let working_directory = native_folder.join("working 文档 e\u{301}");
    assert_ne!(executable, alias);
    assert_eq!(executable.to_string_lossy(), alias.to_string_lossy());
    assert!(executable.to_str().is_none());

    harness.click("Browse comparator executable", AccessibleRole::Button);
    let (request, input) = harness.latest_picker();
    assert_eq!(input.kind, PathPickerKind::Program);
    assert!(view.get_busy());
    harness.event(Event::PathPicked {
        request,
        result: Ok(Some(executable.clone())),
    });
    assert!(!view.get_busy());
    assert_eq!(
        view.get_draft().executable.as_str(),
        executable.to_string_lossy()
    );

    // Cancelling another picker must retain the previous native path, even though
    // reconstructing it from its displayed spelling would lose its identity.
    harness.click("Browse comparator executable", AccessibleRole::Button);
    let (request, input) = harness.latest_picker();
    assert_eq!(input.kind, PathPickerKind::Program);
    assert_eq!(input.initial_directory.as_deref(), executable.parent());
    harness.event(Event::PathPicked {
        request,
        result: Ok(None),
    });
    assert!(!view.get_busy());
    assert_eq!(
        view.get_draft().executable.as_str(),
        executable.to_string_lossy()
    );

    harness.click(
        "Browse comparator working directory",
        AccessibleRole::Button,
    );
    let (request, input) = harness.latest_picker();
    assert_eq!(input.kind, PathPickerKind::Directory);
    harness.event(Event::PathPicked {
        request,
        result: Ok(Some(working_directory.clone())),
    });

    // Exercise the supplied insertion controls rather than accepting the default rows.
    harness.click("Remove argument", AccessibleRole::Button);
    harness.click("Remove argument", AccessibleRole::Button);
    assert!(harness.arguments().is_empty());
    harness.click("Insert left file", AccessibleRole::Button);
    harness.click("Insert right file", AccessibleRole::Button);
    let literal = " --literal=\"文档 two words\" {left} & ";
    harness.click("Add argument", AccessibleRole::Button);
    harness.input("Comparator new argument", literal);
    harness.click("Add argument", AccessibleRole::Button);
    ui::key(&window, Key::Return);
    harness.settle();
    assert!(window.get_options_open());
    assert!(
        harness.saves().is_empty(),
        "argument Enter must stay inside Options"
    );
    assert_eq!(harness.arguments(), ["{left}", "{right}", literal, ""]);

    harness.click("Add comparator rule", AccessibleRole::Button);
    harness.input("Comparator filename pattern", "*.log");
    harness.choose_comparator_target(1);
    harness.click("Enable this comparator rule", AccessibleRole::Checkbox);
    harness.click("Move comparator rule up", AccessibleRole::Button);
    assert_eq!(view.get_selected(), 0);
    assert_eq!(view.get_draft().expression, "*.log");
    assert!(!view.get_draft().enabled);
    harness.select_comparator_row(1);
    assert_eq!(view.get_draft().expression, "*.c;*.cpp");
    assert_eq!(harness.arguments(), ["{left}", "{right}", literal, ""]);
    let launch = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(Arc::ptr_eq(&launch.comparators, &initial.preferences));
    assert_eq!(launch.comparator_policy, initial.id);

    harness.click("OK", AccessibleRole::Button);
    assert!(
        window.get_options_open(),
        "OK must await the save acknowledgement"
    );
    let saves = harness.saves();
    assert_eq!(saves.len(), 1);
    let (revision, submitted) = &saves[0];
    let rules = &submitted.file_comparators.rules;
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].expression, "*.log");
    assert!(!rules[0].enabled);
    assert_eq!(rules[0].target, FileComparatorTarget::BuiltInText);
    assert_eq!(rules[1].expression, "*.c;*.cpp");
    assert!(rules[1].enabled);
    let FileComparatorTarget::External(program) = &rules[1].target else {
        panic!("the second saved rule must retain its external program")
    };
    assert_eq!(program.executable, executable);
    assert_ne!(program.executable, alias);
    assert_eq!(program.working_directory.as_ref(), Some(&working_directory));
    assert_eq!(program.arguments, ["{left}", "{right}", literal, ""]);
    render::snapshot(
        &harness.ui,
        &format!("comparator-workflow-{width}x{height}-font{font}-save-pending"),
    );
    let pending = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(Arc::ptr_eq(&pending.comparators, &initial.preferences));
    assert_eq!(pending.comparator_policy, initial.id);

    // Simulate backend acceptance with the real store's reserved opaque ID and
    // exact raw Arc. The event carries metadata only, never this policy owner.
    let raw = Arc::new(submitted.file_comparators.clone());
    let prepared = PreparedFileComparatorPolicy::new(Arc::new(SavedOptionsPolicy(raw.clone())))
        .with_id(backend.prepare_id().unwrap());
    assert!(
        prepared
            .validation_proof()
            .matches(&submitted.file_comparators)
    );
    let accepted = backend.publish(prepared);
    assert_ne!(accepted.id, initial.id);
    harness.event(Event::PreferencesSaved {
        revision: *revision,
        result: Ok(accepted.clone()),
    });
    assert!(!window.get_options_open());
    let launch = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(Arc::ptr_eq(&launch.comparators, &raw));
    assert_eq!(launch.comparator_policy, accepted.id);

    harness.reopen_comparators();
    assert_eq!(view.get_rule_count(), 2);
    assert!(!view.get_dirty());
    harness.select_comparator_row(0);
    assert_eq!(view.get_draft().expression, "*.log");
    assert_eq!(view.get_draft().target, 1);
    assert!(!view.get_draft().enabled);
    harness.select_comparator_row(1);
    assert_eq!(view.get_draft().expression, "*.c;*.cpp");
    assert_eq!(
        view.get_draft().executable.as_str(),
        executable.to_string_lossy()
    );
    assert_eq!(
        view.get_draft().working_directory.as_str(),
        working_directory.to_string_lossy()
    );
    assert_eq!(harness.arguments(), ["{left}", "{right}", literal, ""]);
    assert!(!view.get_dirty());
    render::snapshot(
        &harness.ui,
        &format!("comparator-workflow-{width}x{height}-font{font}-reopened"),
    );

    harness.input("Comparator filename pattern", "*.discarded");
    harness.input("Comparator executable", "uncommitted relative program");
    harness.click("Comparator argument 1", AccessibleRole::ListItem);
    harness.input("Comparator argument 1", "discard this literal");
    harness.choose_comparator_target(2);
    harness.click("Move comparator rule up", AccessibleRole::Button);
    harness.click("Add comparator rule", AccessibleRole::Button);
    harness.input("Comparator filename pattern", "*.not-saved");
    assert_eq!(view.get_rule_count(), 3);
    harness.click("Cancel", AccessibleRole::Button);
    assert!(!window.get_options_open());
    assert_eq!(
        harness.saves().len(),
        1,
        "Cancel must not emit another settings save"
    );
    let rolled_back = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(Arc::ptr_eq(&rolled_back.comparators, &raw));
    assert_eq!(rolled_back.comparator_policy, accepted.id);

    harness.reopen_comparators();
    assert_eq!(view.get_rule_count(), 2);
    harness.select_comparator_row(0);
    assert_eq!(view.get_draft().expression, "*.log");
    assert!(!view.get_draft().enabled);
    harness.select_comparator_row(1);
    assert_eq!(view.get_draft().expression, "*.c;*.cpp");
    assert_eq!(view.get_draft().target, 3);
    assert_eq!(harness.arguments(), ["{left}", "{right}", literal, ""]);
    render::snapshot(
        &harness.ui,
        &format!("comparator-workflow-{width}x{height}-font{font}-cancel-restored"),
    );
    harness.click("OK", AccessibleRole::Button);
    let saves = harness.saves();
    assert_eq!(saves.len(), 2);
    assert_eq!(saves[1].1.file_comparators, submitted.file_comparators);

    // An unchanged second save keeps the accepted raw Arc and ID as in production.
    let reused = backend
        .reuse_for_save(&saves[1].1.file_comparators)
        .unwrap();
    assert!(
        reused
            .validation_proof()
            .matches(&saves[1].1.file_comparators)
    );
    let unchanged = reused.snapshot();
    drop(reused);
    harness.event(Event::PreferencesSaved {
        revision: saves[1].0,
        result: Ok(unchanged),
    });
    assert!(!window.get_options_open());
    let launch = harness
        .presenter
        .borrow()
        .file_compare_launch(PaneId::Left)
        .unwrap();
    assert!(Arc::ptr_eq(&launch.comparators, &raw));
    assert_eq!(launch.comparator_policy, accepted.id);
    assert!(harness.sent.borrow().iter().all(|command| !matches!(
        command,
        Command::LaunchExternalComparison(_)
            | Command::EditFile { .. }
            | Command::EditCapturedFile { .. }
            | Command::ExecuteUserCommand(_)
    )));
}

#[test]
fn compact_external_comparator_workflow_saves_raw_paths_and_cancel_rolls_back() {
    successful_external_workflow(320, 200, 12, UiDensity::Desktop);
}

#[test]
fn touch_external_comparator_workflow_saves_raw_paths_and_cancel_rolls_back() {
    successful_external_workflow(1100, 740, 32, UiDensity::Touch);
}
