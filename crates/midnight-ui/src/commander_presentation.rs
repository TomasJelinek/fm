//! Small UI bindings for worker-owned directory comparison and panel filtering.
use super::*;

pub(super) fn bind(
    window: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) {
    macro_rules! callback {
        ($name:ident, |$p:ident, $s:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone();
            let send = send.clone();
            let weak = window.as_weak();
            window.$name(move |$($arg),*| {
                let Some(window) = weak.upgrade() else { return; };
                {
                    let mut borrow = state.borrow_mut();
                    let $p = &mut *borrow;
                    let $s = send.as_ref();
                    $body
                }
                refresh_properties(&window, &state.borrow(), false);
            });
        }};
    }
    callback!(on_open_comparison, |p, _s| {
        p.open_comparison();
    });
    callback!(on_panel_split_changed, |p, s, ratio| {
        p.change_panel_split(ratio, s);
    });
    callback!(on_cancel_comparison, |p, s| {
        p.cancel_comparison(s);
    });
    callback!(
        on_submit_comparison,
        |p, s, size, modified, case, tolerance, directories, contents, subfolders| {
            match tolerance.trim().parse::<u32>() {
                Ok(seconds) => p.submit_comparison(
                    CompareOptions {
                        by_size: size,
                        by_modified: modified,
                        by_content: contents,
                        recursive: subfolders,
                        case_sensitive: case,
                        time_tolerance_seconds: seconds,
                        unmatched_directories: directories,
                    },
                    s,
                ),
                Err(_) => {
                    p.comparison.error = "Enter a whole number of seconds from 0 to 86400.".into()
                }
            }
        }
    );
    callback!(on_open_panel_filter, |p, _s, id| {
        if let Some(pane) = match id {
            0 => Some(PaneId::Left),
            1 => Some(PaneId::Right),
            _ => None,
        } {
            p.open_panel_filter(pane);
        }
    });
    callback!(on_clear_panel_filter, |p, s, id| {
        if let Some(pane) = match id {
            0 => Some(PaneId::Left),
            1 => Some(PaneId::Right),
            _ => None,
        } {
            p.clear_panel_filter(pane, s);
        }
    });
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.comparison;
    window.set_comparison_view(ComparisonView {
        open: state.open,
        busy: state.busy,
        // A failed comparison names the path it could not read.
        error: shown_text(&state.error),
        summary: state.summary.as_str().into(),
        left_path: shown_text(&presenter.panes[0].display_path().to_string_lossy()),
        right_path: shown_text(&presenter.panes[1].display_path().to_string_lossy()),
        by_size: state.options.by_size,
        by_modified: state.options.by_modified,
        case_sensitive: state.options.case_sensitive,
        tolerance_seconds: state.options.time_tolerance_seconds.to_string().into(),
        unmatched_directories: state.options.unmatched_directories,
        by_content: state.options.by_content,
        recursive: state.options.recursive,
        progress: if state.busy && state.in_progress() {
            shown_text(&state.progress)
        } else {
            SharedString::default()
        },
    });
    let state = &presenter.filter_dialog;
    let (history, history_labels) = filter_history(window, presenter);
    window.set_panel_filter_view(PanelFilterView {
        history,
        history_labels,
        open: state.open,
        pane: state.pane.index() as i32,
        enabled: state.draft.enabled,
        mode: mode_index(state.draft.mode),
        expression: state.draft.expression.as_str().into(),
        case_sensitive: state.draft.case_sensitive,
        busy: state.busy,
        error: shown_text(&state.error),
    });
    let label = |slot: usize| -> SharedString {
        let filter = &presenter.preferences.values.panel_filters[slot];
        if !filter.enabled {
            return "".into();
        }
        format!(
            "Filter: {}{}",
            match filter.mode {
                FilterMode::Masks => "",
                FilterMode::Regex => "regex ",
                FilterMode::Expression => "expression ",
            },
            filter.expression
        )
        .into()
    };
    window.set_left_filter_label(label(0));
    window.set_right_filter_label(label(1));
}

fn mode_index(mode: FilterMode) -> i32 {
    match mode {
        FilterMode::Masks => 0,
        FilterMode::Regex => 1,
        FilterMode::Expression => 2,
    }
}

type HistoryModels = (
    slint::ModelRc<PanelFilterHistoryItem>,
    slint::ModelRc<SharedString>,
);

/// Reuse the published history models while the history is unchanged. This
/// runs on every refresh (up to 100 entries of 4 KiB); new models would also
/// reset the open filter form's history chooser, since models compare by identity.
fn filter_history(window: &MainWindow, presenter: &Presenter) -> HistoryModels {
    let source = &presenter.preferences.values.panel_filter_history;
    let previous = window.get_panel_filter_view();
    let unchanged = previous.history_labels.row_count() == source.len() + 1
        && previous.history.row_count() == source.len()
        && source.iter().enumerate().all(|(index, filter)| {
            previous.history.row_data(index).is_some_and(|row| {
                row.mode == mode_index(filter.mode)
                    && row.case_sensitive == filter.case_sensitive
                    && row.expression.as_str() == filter.expression
            })
        });
    if unchanged {
        return (previous.history, previous.history_labels);
    }
    let history = source
        .iter()
        .map(|filter| PanelFilterHistoryItem {
            label: format!(
                "{}{}: {}",
                match filter.mode {
                    FilterMode::Masks => "Masks",
                    FilterMode::Regex => "Regex",
                    FilterMode::Expression => "Expression",
                },
                if filter.case_sensitive {
                    " (case sensitive)"
                } else {
                    ""
                },
                filter.expression
            )
            .into(),
            mode: mode_index(filter.mode),
            expression: filter.expression.as_str().into(),
            case_sensitive: filter.case_sensitive,
        })
        .collect::<Vec<_>>();
    let mut labels = vec![SharedString::from(if history.is_empty() {
        "No filter history yet"
    } else {
        "Choose a recent filter…"
    })];
    labels.extend(history.iter().map(|item| item.label.clone()));
    (
        slint::ModelRc::new(slint::VecModel::from(history)),
        slint::ModelRc::new(slint::VecModel::from(labels)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_keeps_filter_history_models_until_the_history_changes() {
        let filter = |expression: &str| PanelFilter {
            enabled: true,
            mode: FilterMode::Masks,
            expression: expression.into(),
            case_sensitive: false,
        };
        let mut h = crate::test_harness::Harness::new(UserPreferences {
            panel_filter_history: vec![filter("*.rs"), filter("*.md")],
            ..Default::default()
        });
        let before = h.window.get_panel_filter_view();
        assert_eq!(before.history_labels.row_count(), 3);
        h.refresh();
        let after = h.window.get_panel_filter_view();
        assert!(
            before.history == after.history && before.history_labels == after.history_labels,
            "an unchanged history keeps its models (they compare by identity)"
        );
        let mut values = h.presenter.borrow().preferences.values.clone();
        values.panel_filter_history[1] = filter("*.toml");
        h.presenter.borrow_mut().preferences.values = values;
        h.refresh();
        let changed = h.window.get_panel_filter_view();
        assert!(changed.history != after.history);
        assert_eq!(
            changed.history.row_data(1).unwrap().expression.as_str(),
            "*.toml"
        );
    }
}
