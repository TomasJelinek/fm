use super::*;
use std::cell::RefCell;

#[test]
fn clearing_filter_preserves_expression_and_rejects_pending_validation() {
    let mut p = presenter();
    p.preferences.values.panel_filters[0] = PanelFilter {
        enabled: true,
        expression: "*.rs".into(),
        ..Default::default()
    };
    p.open_panel_filter(PaneId::Left);
    let request = submit(&mut p, "kind = file");
    p.clear_panel_filter(PaneId::Left, &|_| true);
    assert!(!p.filter_dialog.open);
    assert!(!p.preferences.values.panel_filters[0].enabled);
    assert_eq!(p.preferences.values.panel_filters[0].expression, "*.rs");
    p.apply(Event::PanelFilterValidated {
        request,
        result: Ok(()),
        diagnostic: None,
    });
    p.flush_filters(&|_| panic!("cleared filter must not be restored"));
    assert!(!p.preferences.values.panel_filters[0].enabled);
}

fn presenter() -> Presenter {
    let root = std::env::temp_dir().join("metadata filter 文档");
    let mut p = Presenter::new(root.clone(), root);
    p.preferences.loaded = true;
    p.preferences.values.panel_filters[0] = PanelFilter {
        enabled: true,
        expression: "*.txt".into(),
        ..Default::default()
    };
    p
}
fn submit(p: &mut Presenter, source: &str) -> FilterValidationRequest {
    let sent = RefCell::new(None);
    p.submit_panel_filter(
        PaneId::Left,
        PanelFilter {
            enabled: true,
            mode: FilterMode::Expression,
            expression: source.into(),
            case_sensitive: true,
        },
        &|command| {
            let Command::ValidatePanelFilter(request) = command else {
                panic!("Unexpected command")
            };
            *sent.borrow_mut() = Some(request);
            true
        },
    );
    sent.into_inner().unwrap()
}

#[test]
fn invalid_expression_preserves_last_valid_filter_and_reports_unicode_column() {
    let mut p = presenter();
    let before = p.preferences.values.panel_filters.clone();
    p.open_panel_filter(PaneId::Left);
    let request = submit(&mut p, "name = \"é\" trailing");
    let start = request.filter.expression.find("trailing").unwrap();
    p.apply(Event::PanelFilterValidated {
        request,
        result: Err(FsError::from_io(
            "filter",
            Path::new(""),
            std::io::ErrorKind::InvalidInput.into(),
        )),
        diagnostic: Some(FilterDiagnostic::new(
            start..start + 8,
            FilterDiagnosticKind::Syntax,
            "Unexpected token after expression.",
        )),
    });
    p.flush_filters(&|_| panic!("An invalid expression cannot be saved"));
    assert_eq!(p.preferences.values.panel_filters, before);
    assert!(p.filter_dialog.open);
    assert!(!p.filter_dialog.busy);
    assert!(p.filter_dialog.error.contains("Line 1, column 12"));
    assert_eq!(p.filter_dialog.draft.expression, "name = \"é\" trailing");
}

#[test]
fn cancelled_or_stale_validation_cannot_replace_newer_expression_draft() {
    let mut p = presenter();
    p.open_panel_filter(PaneId::Left);
    let old = submit(&mut p, "kind = directory");
    p.cancel_panel_filter();
    p.open_panel_filter(PaneId::Left);
    let current = submit(&mut p, "kind = file");
    p.apply(Event::PanelFilterValidated {
        request: old,
        result: Ok(()),
        diagnostic: None,
    });
    p.flush_filters(&|_| panic!("Stale validation was applied"));
    assert!(p.filter_dialog.busy);
    assert_eq!(p.filter_dialog.draft.expression, "kind = file");
    p.apply(Event::PanelFilterValidated {
        request: current,
        result: Ok(()),
        diagnostic: None,
    });
    p.flush_filters(&|_| true);
    assert_eq!(
        p.preferences.values.panel_filters[0].mode,
        FilterMode::Expression
    );
    assert_eq!(
        p.preferences.values.panel_filters[0].expression,
        "kind = file"
    );
    assert!(!p.filter_dialog.open);
}

#[test]
fn refused_expression_validation_keeps_the_draft_retryable_without_changing_preferences() {
    let mut p = presenter();
    p.open_panel_filter(PaneId::Left);
    let before = p.preferences.values.panel_filters.clone();
    let filter = PanelFilter {
        enabled: true,
        mode: FilterMode::Expression,
        expression: "size >= 10".into(),
        case_sensitive: false,
    };
    p.submit_panel_filter(PaneId::Left, filter.clone(), &|_| false);
    assert_eq!(p.preferences.values.panel_filters, before);
    assert_eq!(p.filter_dialog.draft, filter);
    assert!(!p.filter_dialog.busy);
    assert!(p.filter_dialog.pending.is_none());
    assert!(!p.filter_dialog.error.is_empty());
    let request = submit(&mut p, "size >= 10");
    assert_eq!(request.id, 1);
}

#[test]
fn captured_show_all_rejects_a_new_navigation_without_touching_its_applied_filter() {
    let mut p = presenter();
    p.open_panel_filter(PaneId::Left);
    let before = p.preferences.values.panel_filters.clone();
    p.panes[0].generation += 1;
    p.clear_captured_panel_filter(&|_| panic!("stale Show all must not enqueue or save"));
    assert!(p.filter_dialog.open);
    assert!(p.filter_dialog.error.contains("panel changed"));
    assert_eq!(p.preferences.values.panel_filters, before);
}

#[test]
fn only_successfully_applied_filters_enter_bounded_deduplicated_history() {
    let mut p = presenter();
    p.open_panel_filter(PaneId::Left);
    let request = submit(&mut p, "kind = file");
    assert!(p.preferences.values.panel_filter_history.is_empty());
    p.apply(Event::PanelFilterValidated {
        request,
        result: Ok(()),
        diagnostic: None,
    });
    assert!(p.preferences.values.panel_filter_history.is_empty());
    p.flush_filters(&|_| true);
    assert_eq!(p.preferences.values.panel_filter_history.len(), 1);
    assert_eq!(
        p.preferences.values.panel_filter_history[0].expression,
        "kind = file"
    );
    p.open_panel_filter(PaneId::Left);
    let request = submit(&mut p, "kind = directory");
    p.cancel_panel_filter();
    p.apply(Event::PanelFilterValidated {
        request,
        result: Ok(()),
        diagnostic: None,
    });
    p.flush_filters(&|_| panic!("cancelled history request cannot save"));
    assert_eq!(p.preferences.values.panel_filter_history.len(), 1);
    let mut history = Vec::new();
    for index in 0..MAX_PANEL_FILTER_HISTORY + 3 {
        remember_history(
            &mut history,
            &PanelFilter {
                enabled: true,
                expression: format!("*.{index}"),
                ..Default::default()
            },
        );
    }
    assert_eq!(history.len(), MAX_PANEL_FILTER_HISTORY);
    let reused = history[20].clone();
    remember_history(&mut history, &reused);
    assert_eq!(history[0], reused);
    assert_eq!(history.iter().filter(|value| **value == reused).count(), 1);
    let mut different_case = reused.clone();
    different_case.case_sensitive = true;
    remember_history(&mut history, &different_case);
    assert_eq!(history[0], different_case);
    assert_eq!(history[1], reused);
    remember_history(
        &mut history,
        &PanelFilter {
            enabled: true,
            expression: String::new(),
            ..Default::default()
        },
    );
    assert_eq!(history[0], different_case);
}
