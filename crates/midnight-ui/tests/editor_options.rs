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
        ["{file}", " --literal=\"文档\" & ", ""]
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
