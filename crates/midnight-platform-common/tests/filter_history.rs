use midnight_core::{FilterMode, PanelFilter, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

#[test]
fn filter_history_round_trips_mode_case_and_order_without_enabling_panel_filters() {
    let history = vec![
        PanelFilter {
            enabled: true,
            mode: FilterMode::Regex,
            expression: "^文档.*".into(),
            case_sensitive: true,
        },
        PanelFilter {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*.txt;*.md".into(),
            case_sensitive: false,
        },
    ];
    let preferences = UserPreferences {
        panel_filter_history: history.clone(),
        ..Default::default()
    };
    let restored = decode_preferences(&encode_preferences(preferences).unwrap()).unwrap();
    assert_eq!(restored.panel_filter_history, history);
    assert!(restored.panel_filters.iter().all(|filter| !filter.enabled));
}

#[test]
fn invalid_duplicate_and_overlong_histories_are_not_persisted() {
    let filter = PanelFilter {
        enabled: true,
        expression: "*.txt".into(),
        ..Default::default()
    };
    let mut preferences = UserPreferences {
        panel_filter_history: vec![filter.clone(); 2],
        ..Default::default()
    };
    assert!(encode_preferences(preferences.clone()).is_err());
    preferences.panel_filter_history = vec![PanelFilter {
        mode: FilterMode::Regex,
        expression: "[".into(),
        ..filter.clone()
    }];
    assert!(encode_preferences(preferences.clone()).is_err());
    preferences.panel_filter_history = (0..101)
        .map(|index| PanelFilter {
            expression: format!("file{index}"),
            ..filter.clone()
        })
        .collect();
    assert!(encode_preferences(preferences).is_err());
}
