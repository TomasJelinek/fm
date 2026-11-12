//! Selection survives focus changes in real detailed, brief and thumbnail rows.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::{FileRow, Theme};
use slint::{ComponentHandle, VecModel};
use std::rc::Rc;

#[test]
fn selected_rows_keep_their_fill_when_focused_in_every_theme_and_view() {
    let mut test = ui::TestUi::new();
    let names = [
        "Ordinary.txt",
        "Caret.txt",
        "Selected.txt",
        "Selected caret.txt",
    ];
    test.window.set_left_rows(
        Rc::new(VecModel::from(
            names
                .iter()
                .enumerate()
                .map(|(state, name)| FileRow {
                    identity: state.to_string().into(),
                    name: (*name).into(),
                    selected: state >= 2,
                    cursor: state == 1 || state == 3,
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    test.window.set_left_search_query("4".into());
    for theme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        test.window.invoke_apply_preferences(theme, 0);
        for active in [true, false] {
            test.window.set_active_pane(if active { 0 } else { 1 });
            for mode in 0..3 {
                test.window.set_left_view_mode(mode);
                test.draw();
                let fills: Vec<_> = names
                    .iter()
                    .map(|name| {
                        let row = ui::root_item(&test.window, name, AccessibleRole::ListItem);
                        let geometry = row.geometry();
                        let origin = row.map_to_native_window(geometry.origin);
                        let p = test.pixel(
                            origin.x + geometry.size.width - 8.0,
                            origin.y + geometry.size.height / 2.0,
                        );
                        [p.r, p.g, p.b]
                    })
                    .collect();
                assert_eq!(
                    fills[2], fills[3],
                    "theme {theme}, active {active}, mode {mode}: focus must preserve selected fill"
                );
                assert_ne!(
                    fills[0], fills[2],
                    "selection must differ from ordinary rows"
                );
                if !matches!(theme, 21..=22) {
                    assert!(
                        (0..3).any(|channel| fills[1][channel].abs_diff(fills[2][channel]) >= 20),
                        "theme {theme}, active {active}, mode {mode}: selected fill must stand apart from caret"
                    );
                }
                let row = ui::root_item(&test.window, names[3], AccessibleRole::ListItem);
                let geometry = row.geometry();
                let origin = row.map_to_native_window(geometry.origin);
                let edge = test.window.global::<Theme>();
                let edge = if active {
                    edge.get_cursor_edge()
                } else {
                    edge.get_selected_edge()
                };
                let bottom = test.pixel(
                    origin.x + geometry.size.width / 2.0,
                    origin.y + geometry.size.height - 0.5,
                );
                assert_eq!(
                    [bottom.r, bottom.g, bottom.b],
                    [edge.red(), edge.green(), edge.blue()],
                    "selected caret needs a visible outline in both panes"
                );
                if mode == 0 && matches!(theme, 0 | 1 | 2 | 21..=25) {
                    render::snapshot(&test, &format!("selection-theme-{theme}-active-{active}"));
                }
            }
        }
    }
    test.window.hide().unwrap();
}
