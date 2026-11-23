//! Small UI bindings for worker-owned directory comparison and panel filtering.
use super::*;
use crate::presenter::comparison::IgnoreChoices;

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
    callback!(on_submit_comparison, |p, s, choices| {
        match comparison_options(&choices) {
            Ok((options, ignore)) => p.submit_comparison(options, ignore, s),
            Err(error) => p.comparison.error = error.into(),
        }
    });
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

/// The dialog's choices as options, and the ignore fields as typed.
fn comparison_options(
    choices: &ComparisonChoices,
) -> Result<(CompareOptions, IgnoreChoices), &'static str> {
    let seconds = choices
        .tolerance_seconds
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|seconds| *seconds <= MAX_COMPARE_TOLERANCE_SECONDS)
        .ok_or("Enter a whole number of seconds from 0 to 86400 for the time tolerance.")?;
    let ignore = IgnoreChoices {
        files: choices.ignore_files,
        files_masks: choices.ignore_files_masks.trim().to_owned(),
        folders: choices.ignore_folders,
        folders_masks: choices.ignore_folders_masks.trim().to_owned(),
    };
    let masks =
        |enabled: bool, masks: &str| (enabled && !masks.is_empty()).then(|| masks.to_owned());
    let options = CompareOptions {
        by_size: choices.by_size,
        by_modified: choices.by_modified,
        by_attributes: choices.by_attributes,
        by_content: choices.by_content,
        recursive: choices.recursive,
        subfolder_attributes: choices.recursive && choices.subfolder_attributes,
        unmatched_directories: choices.unmatched_directories,
        case_sensitive: choices.case_sensitive,
        time_tolerance_seconds: seconds,
        ignore_dst_shifts: choices.ignore_dst,
        ignore_files: masks(ignore.files, &ignore.files_masks),
        ignore_folders: masks(ignore.folders, &ignore.folders_masks),
    };
    Ok((options, ignore))
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.comparison;
    let options = &state.options;
    let running = state.running();
    let (progress_counts, progress_detail, fraction) = if running {
        crate::presenter::comparison::progress_texts(
            state.progress.as_ref(),
            &presenter.number_locale,
        )
    } else {
        (String::new(), String::new(), -1.0)
    };
    let current = state
        .progress
        .as_ref()
        .and_then(|progress| progress.current.clone())
        .filter(|_| running);
    let shown = |path: Option<&std::path::Path>| -> (SharedString, SharedString) {
        let text = path.map(|path| path.to_string_lossy()).unwrap_or_default();
        (shown_text(&text), crate::path_tail(&text))
    };
    let (left_path, left_tail) = shown(Some(presenter.panes[0].display_path()));
    let (right_path, right_tail) = shown(Some(presenter.panes[1].display_path()));
    let (current_left, current_left_tail) = shown(current.as_ref().map(|pair| pair[0].as_path()));
    let (current_right, current_right_tail) = shown(current.as_ref().map(|pair| pair[1].as_path()));
    window.set_comparison_view(ComparisonView {
        open: state.open,
        busy: state.busy,
        running,
        // A failed comparison names the path it could not read.
        error: shown_text(&state.error),
        summary: shown_text(&state.summary),
        left_path,
        left_tail,
        right_path,
        right_tail,
        choices: ComparisonChoices {
            by_size: options.by_size,
            by_modified: options.by_modified,
            by_attributes: options.by_attributes,
            by_content: options.by_content,
            recursive: options.recursive,
            subfolder_attributes: options.subfolder_attributes,
            unmatched_directories: options.unmatched_directories,
            case_sensitive: options.case_sensitive,
            tolerance_seconds: options.time_tolerance_seconds.to_string().into(),
            ignore_dst: options.ignore_dst_shifts,
            ignore_files: state.ignore.files,
            ignore_files_masks: state.ignore.files_masks.as_str().into(),
            ignore_folders: state.ignore.folders,
            ignore_folders_masks: state.ignore.folders_masks.as_str().into(),
        },
        current_left,
        current_left_tail,
        current_right,
        current_right_tail,
        progress_counts: progress_counts.into(),
        progress_detail: progress_detail.into(),
        fraction,
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
/// close the open filter form's history list, since models compare by identity.
fn filter_history(window: &MainWindow, presenter: &Presenter) -> HistoryModels {
    let source = &presenter.preferences.values.panel_filter_history;
    let previous = window.get_panel_filter_view();
    let unchanged = previous.history_labels.row_count() == source.len()
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
    // The rows of the expression line's history list, newest first.
    let labels = history
        .iter()
        .map(|item| item.label.clone())
        .collect::<Vec<SharedString>>();
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
        assert_eq!(before.history_labels.row_count(), 2);
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
