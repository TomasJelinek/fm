use super::*;

fn appearance(background: [u8; 4], foreground: [u8; 4]) -> ViewerAppearance {
    ViewerAppearance {
        family: String::new(),
        font_size: 14.0,
        foreground,
        background,
        selection: [70, 90, 130, 255],
        accent: [200, 120, 255, 255],
    }
}

#[test]
fn readable_keywords_keep_their_saturated_theme_accent() {
    let appearance = appearance([40, 42, 54, 255], [248, 248, 242, 255]);
    assert_eq!(
        role_color(viewer::STYLE_KEYWORD, &appearance),
        appearance.accent
    );
}

#[test]
fn plain_text_remains_the_configured_foreground() {
    let appearance = appearance([40, 42, 54, 255], [248, 248, 242, 255]);
    assert_eq!(role_color(0, &appearance), appearance.foreground);
}

#[test]
fn all_semantic_colors_are_readable_on_light_dark_and_mid_tone_surfaces() {
    let luminance = |rgb: [u8; 4]| {
        rgb[..3]
            .iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(&c, weight)| {
                let c = f64::from(c) / 255.0;
                weight
                    * if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum::<f64>()
    };
    for background in [[255; 4], [40, 42, 54, 255], [128, 128, 128, 255]] {
        let appearance = appearance(background, [248, 248, 242, 255]);
        let palette = Palette::new(&appearance);
        for role in [1, 2, 3, 4, 5, 6, 8, 9, 11] {
            let (fg, bg) = (luminance(palette.color(role)), luminance(background));
            assert!((fg.max(bg) + 0.05) / (fg.min(bg) + 0.05) >= 4.5);
        }
        assert_eq!(palette.color(u16::MAX), appearance.foreground);
    }
}
