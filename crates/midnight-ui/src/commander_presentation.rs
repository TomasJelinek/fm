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
    callback!(on_cancel_comparison, |p, s| {
        p.cancel_comparison(s);
    });
    callback!(
        on_submit_comparison,
        |p, s, size, modified, case, tolerance, directories| {
            match tolerance.trim().parse::<u32>() {
                Ok(seconds) => p.submit_comparison(
                    CompareOptions {
                        by_size: size,
                        by_modified: modified,
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
    callback!(on_cancel_panel_filter, |p, _s| {
        p.cancel_panel_filter();
    });
    callback!(
        on_apply_panel_filter,
        |p, s, id, enabled, mode, expression, case_sensitive| {
            if let Some(pane) = match id {
                0 => Some(PaneId::Left),
                1 => Some(PaneId::Right),
                _ => None,
            } {
                let mode = match mode {
                    0 => FilterMode::Masks,
                    1 => FilterMode::Regex,
                    2 => FilterMode::Expression,
                    _ => {
                        p.filter_dialog.error = "Choose Masks, Regex or Expression.".into();
                        return;
                    }
                };
                p.submit_panel_filter(
                    pane,
                    PanelFilter {
                        enabled,
                        mode,
                        expression: expression.to_string(),
                        case_sensitive,
                    },
                    s,
                );
            }
        }
    );
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.comparison;
    window.set_comparison_view(ComparisonView {
        open: state.open,
        busy: state.busy,
        error: state.error.as_str().into(),
        summary: state.summary.as_str().into(),
        left_path: presenter.panes[0]
            .display_path()
            .to_string_lossy()
            .as_ref()
            .into(),
        right_path: presenter.panes[1]
            .display_path()
            .to_string_lossy()
            .as_ref()
            .into(),
        by_size: state.options.by_size,
        by_modified: state.options.by_modified,
        case_sensitive: state.options.case_sensitive,
        tolerance_seconds: state.options.time_tolerance_seconds.to_string().into(),
        unmatched_directories: state.options.unmatched_directories,
    });
    let state = &presenter.filter_dialog;
    window.set_panel_filter_view(PanelFilterView {
        open: state.open,
        pane: state.pane.index() as i32,
        enabled: state.draft.enabled,
        mode: match state.draft.mode {
            FilterMode::Masks => 0,
            FilterMode::Regex => 1,
            FilterMode::Expression => 2,
        },
        expression: state.draft.expression.as_str().into(),
        case_sensitive: state.draft.case_sensitive,
        busy: state.busy,
        error: state.error.as_str().into(),
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
