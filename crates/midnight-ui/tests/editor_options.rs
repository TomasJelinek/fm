#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
use midnight_core::*;
use midnight_ui::{EditorOptionsView, Presenter, install, refresh};
use slint::{ComponentHandle, platform::Key};
use std::{cell::RefCell, rc::Rc};

#[test]
fn editor_rows_save_literals_once_retain_draft_across_pages_and_consume_f4() {
    let mut h = ui::TestUi::new();
    let root = std::env::current_dir().unwrap();
    let p = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &h.window,
        p.clone(),
        Rc::new(move |c| {
            sent.borrow_mut().push(c);
            true
        }),
    );
    p.borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            editor: EditorPreferences {
                rules: Vec::new(),
                executable: Some(std::env::current_exe().unwrap()),
                arguments: vec!["{file}".into(), "--original".into()],
            },
            ..Default::default()
        }))));
    refresh(&h.window, &p.borrow());
    assert!(
        !h.window
            .global::<EditorOptionsView>()
            .get_draft()
            .use_default,
        "Loading custom editor preferences must invalidate the initial form cache"
    );
    h.window.invoke_show_options();
    h.window.set_options_topic_index(4);
    h.draw();
    ui::popup_item(&h.window, "Editor argument 2", AccessibleRole::ListItem)
        .accessible_action(&AccessibilityAction::Default);
    h.draw();
    ui::popup_item(&h.window, "Editor argument 2", AccessibleRole::TextInput).accessible_action(
        &AccessibilityAction::SetValue(" --literal=\"文档\" & ".into()),
    );
    h.draw();
    assert!(h.window.global::<EditorOptionsView>().get_dirty());
    h.window.set_options_topic_index(0);
    h.draw();
    h.window.set_options_topic_index(4);
    h.draw();
    ui::click_popup(&h.window, "Add argument", AccessibleRole::Button);
    h.draw();
    ui::popup_item(&h.window, "Editor new argument", AccessibleRole::TextInput).accessible_action(
        &AccessibilityAction::SetValue("new literal argument".into()),
    );
    h.draw();
    // A different topic must not prevent OK from validating the retained editor draft.
    h.window.set_options_topic_index(0);
    h.draw();
    ui::click_popup(&h.window, "OK", AccessibleRole::Button);
    h.draw();
    let commands = commands.borrow();
    let values = commands
        .iter()
        .filter_map(|c| match c {
            Command::SavePreferences { preferences, .. } => Some(preferences),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 1);
    assert_eq!(
        values[0].editor.arguments,
        ["{file}", " --literal=\"文档\" & ", "new literal argument"]
    );
    assert!(!h.window.global::<EditorOptionsView>().get_dirty());
    assert!(
        commands
            .iter()
            .all(|c| !matches!(c, Command::EditFile { .. } | Command::ExecuteUserCommand(_)))
    );
    let revision = commands
        .iter()
        .find_map(|command| match command {
            Command::SavePreferences { revision, .. } => Some(*revision),
            _ => None,
        })
        .unwrap();
    drop(commands);
    p.borrow_mut().apply(Event::PreferencesSaved {
        revision,
        result: Ok(()),
    });
    refresh(&h.window, &p.borrow());
    h.window.invoke_show_options();
    h.window.set_options_topic_index(4);
    h.draw();
    let edits = Rc::new(RefCell::new(Vec::new()));
    let recorded = edits.clone();
    h.window
        .on_edit_file(move |pane| recorded.borrow_mut().push(pane));
    ui::click_popup(&h.window, "Editor executable", AccessibleRole::TextInput);
    ui::key(&h.window, Key::F4);
    assert!(
        edits.borrow().is_empty(),
        "F4 while typing must not launch an editor"
    );
}

#[test]
fn ordered_rule_form_retains_fallback_and_cancel_restores_all_associations() {
    let mut h = ui::TestUi::new();
    let root = std::env::current_dir().unwrap();
    let p = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &h.window,
        p.clone(),
        Rc::new(move |command| {
            sent.borrow_mut().push(command);
            true
        }),
    );
    let fallback = std::env::current_exe().unwrap();
    p.borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            editor: EditorPreferences {
                executable: Some(fallback.clone()),
                ..Default::default()
            },
            ..Default::default()
        }))));
    refresh(&h.window, &p.borrow());
    h.window.invoke_show_options();
    h.window.set_options_topic_index(4);
    h.draw();
    let window = h.window.clone_strong();
    let view = window.global::<EditorOptionsView>();
    view.invoke_action(0);
    let mut form = view.get_draft();
    form.expression = "*.txt;*.md".into();
    form.use_default = false;
    form.executable = fallback.to_string_lossy().as_ref().into();
    form.working_directory = root.to_string_lossy().as_ref().into();
    view.invoke_edit(form);
    view.invoke_argument(1, 0, "literal two words".into());
    view.invoke_action(0);
    let mut form = view.get_draft();
    form.mode = 1;
    form.expression = r"\.rs$".into();
    form.enabled = false;
    view.invoke_edit(form);
    view.invoke_action(2);
    assert_eq!(view.get_selected(), 0);
    h.window.set_options_topic_index(0);
    h.draw();
    h.window.set_options_topic_index(4);
    h.draw();
    assert_eq!(view.get_rule_count(), 2);
    ui::click_popup(&h.window, "OK", AccessibleRole::Button);
    h.draw();
    let (saved, revision) = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::SavePreferences {
                preferences,
                revision,
            } => Some((preferences.clone(), *revision)),
            _ => None,
        })
        .unwrap();
    assert_eq!(saved.editor.executable, Some(fallback));
    assert_eq!(saved.editor.rules[0].expression, r"\.rs$");
    assert!(!saved.editor.rules[0].enabled);
    assert_eq!(
        saved.editor.rules[1].arguments,
        ["{file}", "literal two words"]
    );
    assert_eq!(saved.editor.rules[1].working_directory, Some(root));
    p.borrow_mut().apply(Event::PreferencesSaved {
        revision,
        result: Ok(()),
    });
    refresh(&h.window, &p.borrow());
    h.window.invoke_show_options();
    h.window.set_options_topic_index(4);
    h.draw();
    view.invoke_action(0);
    assert_eq!(view.get_rule_count(), 3);
    ui::click_popup(&h.window, "Cancel", AccessibleRole::Button);
    h.draw();
    h.window.invoke_show_options();
    h.window.set_options_topic_index(4);
    h.draw();
    assert_eq!(view.get_rule_count(), 2);
    view.invoke_select(1);
    h.draw();
    render::snapshot(&h, "editor-rules-priority-and-program");
    assert!(commands.borrow().iter().all(|command| !matches!(
        command,
        Command::EditFile { .. } | Command::ExecuteUserCommand(_)
    )));
}
