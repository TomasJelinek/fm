//! The selected-glyph color rule, checked against the WCAG 2 definitions.
use super::*;

const OPAQUE: u8 = 255;
/// No selection text color: each selected glyph keeps its own color.
const KEEP: [u8; 4] = [0; 4];

/// The channel order and the middle channel's relative position between the
/// extremes: together the HSL hue, independent of lightness and saturation.
fn hue(color: [u8; 4]) -> ([usize; 3], f64) {
    let mut order = [0, 1, 2];
    order.sort_by_key(|&channel| color[channel]);
    let [min, mid, max] = order.map(|channel| f64::from(color[channel]));
    (order, (mid - min) / (max - min))
}

fn assert_same_hue(original: [u8; 4], adjusted: [u8; 4]) {
    let ((order, position), (adjusted_order, adjusted_position)) = (hue(original), hue(adjusted));
    assert_eq!(order[2], adjusted_order[2], "{original:?} -> {adjusted:?}");
    assert_eq!(order[0], adjusted_order[0], "{original:?} -> {adjusted:?}");
    assert!(
        (position - adjusted_position).abs() < 0.05,
        "{original:?} -> {adjusted:?}"
    );
}

fn lum(color: [u8; 4]) -> f64 {
    luminance([color[0], color[1], color[2]])
}

#[test]
fn ratio_matches_published_wcag_values_and_ignores_alpha() {
    assert!((ratio([0, 0, 0, OPAQUE], [255; 4]) - 21.0).abs() < 1e-9);
    assert!((ratio([255; 4], [255; 4]) - 1.0).abs() < 1e-12);
    // #767676 is the lightest gray that passes AA on white; #777777 fails.
    assert!((ratio([118, 118, 118, OPAQUE], [255; 4]) - 4.54).abs() < 0.005);
    assert!((ratio([119, 119, 119, OPAQUE], [255; 4]) - 4.48).abs() < 0.005);
    assert_eq!(
        ratio([10, 20, 30, 0], [200; 4]),
        ratio([10, 20, 30, OPAQUE], [200; 4])
    );
}

#[test]
fn readable_and_invisible_brushes_and_transparent_selections_are_kept() {
    let mut colors = SelectionColors::default();
    assert!(!colors.prepare([200, 30, 30, 0], [24, 27, 34, OPAQUE], KEEP));
    assert!(colors.prepare([38, 79, 120, OPAQUE], [24, 27, 34, OPAQUE], KEEP));
    for brush in [
        [230, 230, 200, OPAQUE],
        [255; 4],
        [255, 255, 255, 200],
        [86, 156, 214, 0],
    ] {
        assert_eq!(colors.brush(brush), brush);
    }
}

#[test]
fn a_dark_selection_lightens_tokens_just_enough_and_keeps_their_hue() {
    let selection = [38, 79, 120, OPAQUE];
    let mut colors = SelectionColors::default();
    assert!(colors.prepare(selection, [24, 27, 34, OPAQUE], KEEP));
    for token in [
        [30, 50, 150, OPAQUE],
        [86, 156, 214, OPAQUE],
        [106, 153, 85, OPAQUE],
        [206, 145, 120, OPAQUE],
        [200, 40, 40, OPAQUE],
        [0, 0, 255, OPAQUE],
    ] {
        let adjusted = colors.brush(token);
        let contrast = ratio(adjusted, selection);
        assert!(
            (MINIMUM..MINIMUM + 0.1).contains(&contrast),
            "{token:?} -> {adjusted:?}: {contrast}"
        );
        assert!(lum(adjusted) > lum(token), "{token:?} -> {adjusted:?}");
        assert_same_hue(token, adjusted);
        assert_eq!(adjusted[3], OPAQUE);
        assert_eq!(colors.brush(token), adjusted, "cached");
    }
    let gray = colors.brush([90, 90, 90, OPAQUE]);
    assert!(
        gray[0] == gray[1] && gray[1] == gray[2] && gray[0] > 90,
        "{gray:?}"
    );
}

#[test]
fn a_light_selection_darkens_tokens_instead_of_lightening_them() {
    let selection = [173, 214, 255, OPAQUE];
    let mut colors = SelectionColors::default();
    assert!(colors.prepare(selection, [250, 250, 250, OPAQUE], KEEP));
    for token in [
        [80, 140, 220, OPAQUE],
        [220, 220, 170, OPAQUE],
        [255, 140, 0, OPAQUE],
        [40, 160, 60, OPAQUE],
    ] {
        let adjusted = colors.brush(token);
        let contrast = ratio(adjusted, selection);
        assert!(
            (MINIMUM..MINIMUM + 0.1).contains(&contrast),
            "{token:?} -> {adjusted:?}: {contrast}"
        );
        assert!(lum(adjusted) < lum(token), "{token:?} -> {adjusted:?}");
        assert_same_hue(token, adjusted);
    }
}

#[test]
fn the_nearer_lightness_wins_when_both_directions_reach_the_contrast() {
    // A mid gray selection: only near-white and near-black text reach 4.5:1.
    let selection = [118, 118, 118, OPAQUE];
    let mut colors = SelectionColors::default();
    assert!(colors.prepare(selection, [0, 0, 0, OPAQUE], KEEP));
    let lighter = colors.brush([150, 150, 150, OPAQUE]);
    let darker = colors.brush([105, 105, 105, OPAQUE]);
    assert!(lum(lighter) > lum(selection) && ratio(lighter, selection) >= MINIMUM);
    assert!(lum(darker) < lum(selection) && ratio(darker, selection) >= MINIMUM);
}

#[test]
fn a_translucent_selection_is_judged_as_painted_over_the_viewport() {
    let mut colors = SelectionColors::default();
    assert!(colors.prepare([0, 0, 255, 128], [255, 255, 255, OPAQUE], KEEP));
    assert_eq!(colors.background, [127, 127, 255]);
    let token = [120, 120, 250, OPAQUE];
    let adjusted = colors.brush(token);
    assert!(ratio(adjusted, [127, 127, 255, OPAQUE]) >= MINIMUM);
    assert!(lum(adjusted) < lum(token));
    assert_same_hue(token, adjusted);
    // Another background restarts the cache: the same token is now readable.
    assert!(colors.prepare([30, 30, 30, OPAQUE], [255, 255, 255, OPAQUE], KEEP));
    assert_eq!(colors.brush(token), token);
}

#[test]
fn unreachable_contrast_uses_the_better_of_black_and_white_with_the_brush_alpha() {
    // Painted at alpha 100 over gray 140, black shows as gray 85 (about 2.2:1)
    // and white as gray 185 (about 1.7:1); over gray 60, black shows as gray 36
    // (1.4:1) and white as gray 136 (3.1:1). No lightness reaches 4.5:1.
    let mut colors = SelectionColors::default();
    assert!(colors.prepare([140, 140, 140, OPAQUE], [0, 0, 0, OPAQUE], KEEP));
    assert_eq!(colors.brush([200, 60, 60, 100]), [0, 0, 0, 100]);
    assert!(colors.prepare([60, 60, 60, OPAQUE], [0, 0, 0, OPAQUE], KEEP));
    assert_eq!(colors.brush([200, 60, 60, 100]), [255, 255, 255, 100]);
}

#[test]
fn a_selection_text_color_replaces_every_glyph_color_and_moves_only_when_unreadable() {
    let (selection, background) = ([38, 79, 120, OPAQUE], [24, 27, 34, OPAQUE]);
    let white = [255, 255, 255, OPAQUE];
    let mut colors = SelectionColors::default();
    assert!(colors.prepare(selection, background, white));
    // Dark, light and saturated glyph colors alike: the theme's selected text color.
    for token in [
        [30, 50, 150, OPAQUE],
        [230, 230, 200, OPAQUE],
        [200, 40, 40, OPAQUE],
    ] {
        assert_eq!(colors.brush(token), white, "{token:?}");
    }
    // Glyphs keep their own opacity: an invisible glyph stays invisible.
    assert_eq!(colors.brush([10, 20, 30, 0])[3], 0);
    assert_eq!(colors.brush([10, 20, 30, 128])[3], 128);
    // A text color below 4.5:1 on the fill moves in lightness only, as glyph colors do.
    let dim = [60, 90, 130, OPAQUE];
    assert!(ratio(dim, selection) < MINIMUM);
    assert!(colors.prepare(selection, background, dim));
    let adjusted = colors.brush([200, 40, 40, OPAQUE]);
    assert!(
        (MINIMUM..MINIMUM + 0.1).contains(&ratio(adjusted, selection)),
        "{adjusted:?}"
    );
    assert_same_hue(dim, adjusted);
    // Without a text color each glyph keeps its own readable color again.
    assert!(colors.prepare(selection, background, KEEP));
    assert_eq!(
        colors.brush([230, 230, 200, OPAQUE]),
        [230, 230, 200, OPAQUE]
    );
}

#[test]
fn hsl_round_trips_every_hue_sextant() {
    for color in [
        [255, 0, 0],
        [200, 180, 20],
        [20, 200, 40],
        [10, 190, 200],
        [30, 50, 150],
        [180, 40, 200],
        [200, 30, 90],
        [128, 128, 128],
        [0, 0, 0],
        [255, 255, 255],
    ] {
        let [hue, saturation, lightness] = hsl(color);
        assert!((0.0..6.0).contains(&hue) && (0.0..=1.0).contains(&saturation));
        assert_eq!(rgb(hue, saturation, lightness), color);
    }
    assert_eq!(hsl([255, 0, 0]), [0.0, 1.0, 0.5]);
    assert_eq!(hsl([0, 0, 255]), [4.0, 1.0, 0.5]);
}
