//! Filename-color priority is visible, selectable and reversible inside Options.
#[path = "support/popup_scroll.rs"]
mod popup_scroll;
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    layout::Orientation,
    window::{WindowInner, WindowKind},
};
use midnight_core::{Command, Event, FilenameColorRule, UserPreferences};
use midnight_ui::{ColorOptionsView, Presenter, install, refresh};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl Harness {
    fn new(expressions: &[String]) -> Self {
        Self::with_rules(
            expressions
                .iter()
                .map(|expression| FilenameColorRule {
                    expression: expression.clone(),
                    ..Default::default()
                })
                .collect(),
            None,
        )
    }

    fn with_rules(rules: Vec<FilenameColorRule>, clock: Option<Rc<Cell<Duration>>>) -> Self {
        let ui = ui::TestUi::with_clock(clock);
        let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
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
            .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
                filename_colors: rules.into(),
                ..Default::default()
            }))));
        refresh(&ui.window, &presenter.borrow());
        ui.window.invoke_show_options();
        ui.window.set_options_topic_index(8);
        let mut h = Self {
            ui,
            presenter,
            sent,
        };
        h.settle();
        h
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.ui.draw();
        }
    }

    fn row(&self, label: &str) -> ItemRc {
        ui::popup_item(&self.ui.window, label, AccessibleRole::ListItem)
    }

    fn click(&mut self, label: &str, role: AccessibleRole) {
        popup_scroll::click(&mut self.ui, label, role);
    }

    fn assert_draft_only(&self) {
        assert!(
            !self
                .sent
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::SavePreferences { .. })),
            "editing rule priority must stay inside the Options draft"
        );
    }

    fn labels(&self) -> Vec<String> {
        self.ui
            .window
            .global::<ColorOptionsView>()
            .get_labels()
            .iter()
            .map(|label| label.to_string())
            .collect()
    }
}

#[test]
fn default_extension_groups_remain_responsive_to_keyboard_navigation() {
    let rules = midnight_core::default_filename_colors();
    let last = rules.len() - 1;
    let time = Rc::new(Cell::new(Duration::ZERO));
    let mut h = Harness::with_rules(rules.clone(), Some(time.clone()));
    let labels = h.labels();
    h.click(&labels[0], AccessibleRole::ListItem);

    // Match the reported workflow with the real long default extension group
    // and an actual hover tooltip still present when keyboard navigation starts.
    let first = h.row(&labels[0]);
    let area = bounds(&first);
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerExited);
    h.settle();
    h.ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new((area.0 + area.2) / 2.0, (area.1 + area.3) / 2.0),
        });
    h.settle();
    time.set(Duration::from_secs(2));
    h.settle();
    {
        let inner = WindowInner::from_pub(h.ui.window.window());
        let popups = inner.active_popups();
        let tooltip = popups
            .iter()
            .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("hovering the extension group shows its actual tooltip");
        let text = ui::visible_text_values(ItemRc::new_root(tooltip.component.clone()));
        assert!(
            text.iter()
                .any(|text| text.contains(rules[0].expression.as_str())),
            "the complete extension group must remain available in the tooltip"
        );
    }

    let assert_current = |h: &Harness, index: usize| {
        let colors = h.ui.window.global::<ColorOptionsView>();
        assert_eq!(colors.get_selected(), index as i32);
        let draft = colors.get_draft();
        assert_eq!(draft.expression.as_str(), rules[index].expression);
        assert_eq!(
            draft.foreground.as_str(),
            rules[index].style.foreground.unwrap().hex(),
            "the selected rule's color must follow its extension expression"
        );
        let row = h.row(&labels[index]);
        assert_selected(&row, true);
        let clip = bounds(&scroll_parent(&row));
        let row = bounds(&row);
        assert!(
            row.1 >= clip.1 - 0.5 && row.3 <= clip.3 + 0.5,
            "keyboard selection must be visible: index={index}, row={row:?}, clip={clip:?}"
        );
    };
    assert_current(&h, 0);
    for _ in 0..2 {
        // Down first reproduces the reported transition from source-code
        // extensions to video extensions, then crosses the list's viewport.
        for index in 1..rules.len() {
            ui::key(&h.ui.window, Key::DownArrow);
            h.settle();
            assert_current(&h, index);
        }
        ui::key(&h.ui.window, Key::DownArrow);
        h.settle();
        assert_current(&h, last);
        for index in (0..last).rev() {
            ui::key(&h.ui.window, Key::UpArrow);
            h.settle();
            assert_current(&h, index);
        }
        ui::key(&h.ui.window, Key::UpArrow);
        h.settle();
        assert_current(&h, 0);
        for (key, index) in [
            (Key::End, last),
            (Key::End, last),
            (Key::Home, 0),
            (Key::Home, 0),
        ] {
            ui::key(&h.ui.window, key);
            h.settle();
            assert_current(&h, index);
        }
    }
    h.assert_draft_only();
    ui::key(&h.ui.window, Key::Escape);
    h.settle();
    assert!(
        !h.ui.window.get_options_open(),
        "Options must still close after navigation"
    );
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

fn scroll_parent(item: &ItemRc) -> ItemRc {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    loop {
        let item = parent.expect("color controls must belong to a scroll view");
        if item.downcast::<Flickable>().is_some() {
            return item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

fn assert_selected(row: &ItemRc, selected: bool) {
    assert_eq!(
        row.accessible_string_property(AccessibleStringProperty::ItemSelectable)
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        row.accessible_string_property(AccessibleStringProperty::ItemSelected)
            .as_deref(),
        Some(if selected { "true" } else { "false" })
    );
}

#[test]
fn rule_labels_preserve_the_complete_expression_for_layout_and_accessibility() {
    let expression = format!("{}終端*.txt", "文档*.rs;".repeat(18));
    let h = Harness::new(std::slice::from_ref(&expression));
    let expected = format!("1. {expression}");
    assert_eq!(h.labels().as_slice(), std::slice::from_ref(&expected));
    assert_eq!(
        h.row(&expected)
            .accessible_string_property(AccessibleStringProperty::Label)
            .as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn compact_rule_actions_share_the_heading_row_above_the_priority_list() {
    let mut h = Harness::new(&["*.rs".into(), "*.txt".into()]);
    for (width, height, font) in [(1340, 820, 11), (640, 420, 18)] {
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(width, height);
        h.settle();
        let row = bounds(&h.row("1. *.rs"));
        let mut previous = None;
        for label in ["Add rule", "Remove rule", "Move up", "Move down"] {
            let button = bounds(&ui::popup_item(&h.ui.window, label, AccessibleRole::Button));
            assert!(
                button.3 <= row.1,
                "{label} belongs beside the heading above the list: {button:?}, row={row:?}"
            );
            assert!(
                button.2 - button.0 <= 60.0,
                "{label} must remain a compact action"
            );
            if let Some(previous) = previous {
                assert!(
                    button.0 >= previous,
                    "rule action controls must not overlap"
                );
            }
            previous = Some(button.2);
        }
        render::snapshot(&h.ui, &format!("colors-compact-actions-{width}-{font}"));
    }
}

#[test]
fn ordered_rows_select_reorder_toggle_and_cancel_without_saving() {
    let mut h = Harness::new(&["*.rs".into(), "文档*".into(), "*".into()]);
    let list = ui::popup_item(&h.ui.window, "Filename color rules", AccessibleRole::List);
    assert_eq!(
        list.accessible_string_property(AccessibleStringProperty::ItemCount)
            .as_deref(),
        Some("3")
    );
    let first = h.row("1. *.rs");
    let second = h.row("2. 文档*");
    let third = h.row("3. *");
    assert!(bounds(&first).3 <= bounds(&second).1 + 0.5);
    assert!(bounds(&second).3 <= bounds(&third).1 + 0.5);
    assert_selected(&first, true);
    assert_selected(&second, false);
    h.click("2. 文档*", AccessibleRole::ListItem);
    assert_selected(&h.row("2. 文档*"), true);
    assert_eq!(
        h.ui.window
            .global::<ColorOptionsView>()
            .get_draft()
            .expression,
        "文档*"
    );
    h.click("Move up", AccessibleRole::Button);
    assert_eq!(h.labels(), ["1. 文档*", "2. *.rs", "3. *"]);
    assert_selected(&h.row("1. 文档*"), true);
    h.click("Enable rule", AccessibleRole::Checkbox);
    assert_selected(&h.row("1. [off] 文档*"), true);
    h.click("Move down", AccessibleRole::Button);
    assert_eq!(h.labels(), ["1. *.rs", "2. [off] 文档*", "3. *"]);
    h.click("Add rule", AccessibleRole::Button);
    assert_selected(&h.row("4. *.*"), true);
    h.click("Remove rule", AccessibleRole::Button);
    assert_selected(&h.row("3. *"), true);
    h.assert_draft_only();
    render::snapshot(&h.ui, "colors-priority-list");
    h.click("Cancel", AccessibleRole::Button);
    h.assert_draft_only();
    h.ui.window.invoke_show_options();
    h.ui.window.set_options_topic_index(8);
    h.settle();
    assert_eq!(h.labels(), ["1. *.rs", "2. 文档*", "3. *"]);
    assert!(!h.ui.window.global::<ColorOptionsView>().get_dirty());
}

#[test]
fn accessible_selection_and_reorder_reach_validation_and_ok_in_the_displayed_order() {
    let mut h = Harness::new(&["*.rs".into(), "文档*".into(), "*".into()]);
    h.row("3. *")
        .accessible_action(&AccessibilityAction::Default);
    h.settle();
    assert_selected(&h.row("3. *"), true);
    ui::key(&h.ui.window, Key::UpArrow);
    h.settle();
    assert_selected(&h.row("2. 文档*"), true);
    h.click("Move up", AccessibleRole::Button);
    h.click("OK", AccessibleRole::Button);
    h.assert_draft_only();
    let request = h
        .sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .expect("OK validates the reordered rules before persistence");
    assert_eq!(
        request
            .rules
            .iter()
            .map(|rule| rule.expression.as_str())
            .collect::<Vec<_>>(),
        ["文档*", "*.rs", "*"]
    );
    let captured = h.sent.clone();
    h.presenter.borrow_mut().apply_with_sender(
        Event::FilenameColorsValidated {
            request,
            result: Ok(()),
        },
        &|command| {
            captured.borrow_mut().push(command);
            true
        },
    );
    refresh(&h.ui.window, &h.presenter.borrow());
    let saved = h
        .sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences.clone()),
            _ => None,
        })
        .expect("validated OK saves preferences");
    assert_eq!(
        saved
            .filename_colors
            .iter()
            .map(|rule| rule.expression.as_str())
            .collect::<Vec<_>>(),
        ["文档*", "*.rs", "*"]
    );
}

#[test]
fn priority_keyboard_reveals_last_rule_and_compact_editor_scrolls_above_the_footer() {
    let expressions: Vec<_> = (1..=32).map(|index| format!("rule-{index}.*")).collect();
    let mut h = Harness::new(&expressions);
    h.click("1. rule-1.*", AccessibleRole::ListItem);
    ui::key(&h.ui.window, Key::End);
    h.settle();
    let last = h.row("32. rule-32.*");
    let viewport = scroll_parent(&last);
    assert_selected(&last, true);
    assert!(bounds(&last).1 >= bounds(&viewport).1 - 0.5);
    assert!(bounds(&last).3 <= bounds(&viewport).3 + 0.5);
    ui::key(&h.ui.window, Key::Home);
    h.settle();
    let first = h.row("1. rule-1.*");
    assert_selected(&first, true);
    assert!(bounds(&first).1 >= bounds(&viewport).1 - 0.5);
    assert!(bounds(&first).3 <= bounds(&viewport).3 + 0.5);
    for (density, font) in [(0, 11), (1, 18)] {
        h.ui.window.invoke_apply_preferences(1, density);
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(640, 420);
        h.settle();
        let adapter: Rc<dyn WindowAdapter> = h.ui.native.clone();
        for state in ["Normal", "Focused", "Selected", "Focused + selected"] {
            let text = ui::popup_item(
                &h.ui.window,
                &format!("{state}\nExample.txt"),
                AccessibleRole::Text,
            );
            let required = text
                .borrow()
                .as_ref()
                .layout_info(
                    Orientation::Vertical,
                    text.geometry().width(),
                    &adapter,
                    &text,
                )
                .preferred;
            assert!(
                text.geometry().height() + 0.5 >= required,
                "preview text must fit its card: {state}, density={density}, font={font}, actual={}, required={required}",
                text.geometry().height()
            );
        }
        let apply = ui::popup_item(&h.ui.window, "Apply colors", AccessibleRole::Button);
        let content = scroll_parent(&apply);
        let clip = bounds(&content);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(clip.0 + 3.0, clip.1 + 3.0),
                delta_x: 0.0,
                delta_y: -100_000.0,
            });
        h.settle();
        let bottom = bounds(&apply);
        let ok = bounds(&ui::popup_item(&h.ui.window, "OK", AccessibleRole::Button));
        assert!(bottom.1 >= clip.1, "editor action must scroll into view");
        assert!(
            bottom.3 <= clip.3 - 14.0,
            "keep action above scrollbar overlay"
        );
        assert!(
            bottom.3 < ok.1,
            "editor must remain above fixed Options footer"
        );
        assert!(ok.3 <= h.ui.height as f32, "OK must stay inside the window");
        render::snapshot(&h.ui, &format!("colors-priority-compact-{density}-{font}"));
    }
    h.assert_draft_only();
}
