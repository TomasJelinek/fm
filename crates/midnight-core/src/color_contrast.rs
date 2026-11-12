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
    if contrast(foreground, background) >= 4.5 {
        return foreground;
    }
    let mut best = None;
    for target in [0, 255] {
        if contrast([target; 3], background) < 4.5 {
            continue;
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..14 {
            let middle = (low + high) * 0.5;
            if contrast(mix(foreground, target, middle), background) >= 4.5 {
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
    best.map_or(foreground, |(color, _)| color)
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
}
