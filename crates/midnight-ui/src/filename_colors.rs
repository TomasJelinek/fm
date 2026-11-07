//! Constant-size presentation math shared with viewer syntax colors.
use slint::Color;

pub(crate) fn readable(foreground: Color, background: Color) -> Color {
    let rgb = [foreground.red(), foreground.green(), foreground.blue()];
    let result = midnight_core::readable_rgb(
        rgb,
        [background.red(), background.green(), background.blue()],
    );
    if result == rgb {
        foreground
    } else {
        Color::from_rgb_u8(result[0], result[1], result[2])
    }
}

pub(crate) fn install(window: &crate::MainWindow) {
    use slint::ComponentHandle;
    window
        .global::<crate::FilenameColorPresentation>()
        .on_readable(readable);
}
