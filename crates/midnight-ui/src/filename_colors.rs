//! Constant-size presentation math shared by row painting and the settings preview.
use slint::Color;

fn luminance(color: Color) -> f64 {
    let linear = |channel| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

fn contrast(first: Color, second: Color) -> f64 {
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn mix(color: Color, target: u8, amount: f64) -> Color {
    let channel =
        |value| (f64::from(value) * (1.0 - amount) + f64::from(target) * amount).round() as u8;
    Color::from_rgb_u8(
        channel(color.red()),
        channel(color.green()),
        channel(color.blue()),
    )
}

/// Keep already-readable colors unchanged; adjust lightness toward white or black
/// with the smallest RGB movement. Blending all channels preserves the base hue.
pub(crate) fn readable(foreground: Color, background: Color) -> Color {
    if contrast(foreground, background) >= 4.5 {
        return foreground;
    }
    let mut best = None;
    for target in [0, 255] {
        if contrast(mix(foreground, target, 1.0), background) < 4.5 {
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
        let distance = [
            (result.red(), foreground.red()),
            (result.green(), foreground.green()),
            (result.blue(), foreground.blue()),
        ]
        .into_iter()
        .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum::<f64>();
        if best.is_none_or(|(_, previous)| distance < previous) {
            best = Some((result, distance));
        }
    }
    best.map_or(foreground, |(color, _)| color)
}

pub(crate) fn install(window: &crate::MainWindow) {
    use slint::ComponentHandle;
    window
        .global::<crate::FilenameColorPresentation>()
        .on_readable(readable);
}
