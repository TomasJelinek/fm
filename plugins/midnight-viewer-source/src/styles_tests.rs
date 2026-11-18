use super::*;
use midnight_plugin_sdk::api::viewer::{STYLE_STRING, STYLE_TYPE};

#[test]
fn state_only_watch_observes_a_far_prior_token_even_when_it_resolves_plain() {
    let mut styles = Styles::new(0..0, false);
    styles.origin(100_000);
    styles.watch(5);
    styles.push_pending(100_010, 2, 10, STYLE_PLAIN);
    assert_eq!(
        styles.take_resolution(),
        Some(RoleHint {
            start: 0,
            end: 100_008,
            role: STYLE_PLAIN
        })
    );
    assert!(styles.finish().0.is_empty());
}

#[test]
fn reserved_role_hints_fill_or_override_only_the_exact_visible_range() {
    let mut styles = Styles::new(10..30, false);
    styles.push(10, 30, STYLE_STRING);
    let (spans, window, clipped) = styles.finish_with_hints([
        Some(RoleHint {
            start: 14,
            end: 20,
            role: STYLE_TYPE,
        }),
        None,
    ]);
    assert_eq!(window, 10..30);
    assert!(!clipped);
    assert_eq!(
        spans,
        vec![
            Span {
                start: 10,
                end: 14,
                role: STYLE_STRING,
                flags: 0
            },
            Span {
                start: 14,
                end: 20,
                role: STYLE_TYPE,
                flags: 0
            },
            Span {
                start: 20,
                end: 30,
                role: STYLE_STRING,
                flags: 0
            }
        ]
    );
}

#[test]
fn hints_obey_the_same_directional_span_budget_without_clearing_styles() {
    for backwards in [false, true] {
        let mut styles = Styles::new(0..20_000, backwards);
        for at in 0..MAX_SPANS {
            styles.push(at * 4, at * 4 + 1, STYLE_STRING);
        }
        let (spans, window, clipped) = styles.finish_with_hints([
            Some(RoleHint {
                start: 2,
                end: 3,
                role: STYLE_TYPE,
            }),
            None,
        ]);
        assert_eq!(spans.len(), MAX_SPANS);
        assert!(clipped);
        assert!(spans.windows(2).all(|pair| pair[0].end <= pair[1].start));
        if backwards {
            assert_eq!(window.end, 20_000);
        } else {
            assert_eq!(window.start, 0);
        }
    }
}
