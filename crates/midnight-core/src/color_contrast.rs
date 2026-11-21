//! Shared opaque sRGB text contrast policy for panels and viewer surfaces.

fn luminance(color: [u8; 3]) -> f64 {
    color
        .into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(channel, weight)| {
            let value = f64::from(channel) / 255.0;
            weight
                * if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

fn contrast(first: [u8; 3], second: [u8; 3]) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn mix(color: [u8; 3], target: u8, amount: f64) -> [u8; 3] {
    color
        .map(|value| (f64::from(value) * (1.0 - amount) + f64::from(target) * amount).round() as u8)
}

/// Preserve readable colors; otherwise choose the least RGB movement toward
/// black or white that reaches a 4.5:1 contrast against the opaque background.
pub fn readable_rgb(foreground: [u8; 3], background: [u8; 3]) -> [u8; 3] {
    readable_rgb_at(foreground, background, 4.5)
}

/// The contrast that content colors (syntax roles, links) keep against a page with
/// `foreground` text on `background`: 5:1, a margin over WCAG AA's 4.5:1, and 7:1
/// (WCAG AAA) where the body text itself reaches 15:1 or more, as in the Contrast and
/// High Contrast themes.
pub fn content_minimum(foreground: [u8; 3], background: [u8; 3]) -> f64 {
    if contrast(foreground, background) >= 15.0 {
        7.0
    } else {
        5.0
    }
}

/// [`readable_rgb`] for `minimum` contrast. When neither black nor white reaches it,
/// the better of the two.
pub fn readable_rgb_at(foreground: [u8; 3], background: [u8; 3], minimum: f64) -> [u8; 3] {
    if contrast(foreground, background) >= minimum {
        return foreground;
    }
    let mut best = None;
    for target in [0, 255] {
        if contrast([target; 3], background) < minimum {
            continue;
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..14 {
            let middle = (low + high) * 0.5;
            if contrast(mix(foreground, target, middle), background) >= minimum {
                high = middle;
            } else {
                low = middle;
            }
        }
        let result = mix(foreground, target, high);
        let distance = result
            .into_iter()
            .zip(foreground)
            .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
            .sum::<f64>();
        if best.is_none_or(|(_, previous)| distance < previous) {
            best = Some((result, distance));
        }
    }
    best.map_or_else(
        || {
            if contrast([255; 3], background) >= contrast([0; 3], background) {
                [255; 3]
            } else {
                [0; 3]
            }
        },
        |(color, _)| color,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjustments_are_readable_on_dark_light_and_intermediate_backgrounds() {
        for background in [
            [24, 27, 34],
            [40, 42, 54],
            [255; 3],
            [128; 3],
            [160, 100, 130],
        ] {
            for foreground in [[255, 88, 160], [130, 220, 90], [60, 120, 220], [128; 3]] {
                let adjusted = readable_rgb(foreground, background);
                assert!(contrast(adjusted, background) >= 4.5);
                if contrast(foreground, background) >= 4.5 {
                    assert_eq!(adjusted, foreground);
                }
            }
        }
        assert!((contrast([255; 3], [0; 3]) - 21.0).abs() < 0.000_001);
    }

    #[test]
    fn content_colors_keep_a_margin_and_reach_seven_to_one_on_high_contrast_pages() {
        // Body text of an ordinary light and dark theme, and of the contrast themes.
        assert_eq!(content_minimum([48; 3], [255; 3]), 5.0);
        assert_eq!(content_minimum([237, 241, 245], [31, 34, 40]), 5.0);
        assert_eq!(content_minimum([22, 35, 52], [253, 254, 254]), 7.0);
        assert_eq!(content_minimum([255; 3], [0; 3]), 7.0);
        for background in [[24, 27, 34], [253, 254, 254], [0; 3], [255; 3]] {
            for foreground in [[130, 162, 210], [150, 220, 95], [255, 105, 105]] {
                for minimum in [5.0, 7.0] {
                    let adjusted = readable_rgb_at(foreground, background, minimum);
                    assert!(contrast(adjusted, background) >= minimum);
                }
            }
        }
        // Neither black nor white reaches 7:1 on middle gray: the better of the two.
        assert_eq!(readable_rgb_at([120; 3], [128; 3], 7.0), [0; 3]);
    }
}
