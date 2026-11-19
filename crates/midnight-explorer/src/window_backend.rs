//! Renderer policy shared by startup paths; native window options stay per OS.

#[cfg(target_os = "linux")]
#[path = "window_backend_linux.rs"]
mod native;
#[cfg(windows)]
#[path = "window_backend_windows.rs"]
mod native;

/// How the default renderer fits text to the pixel grid. Both fittings are
/// opt-in while they are being evaluated: `MIDNIGHT_GLYPH_FITTING=snapped` or
/// `fitted`; anything else keeps the renderer's own subpixel placement.
fn glyph_fitting(choice: Option<&str>) -> femtovg::GlyphFitting {
    match choice {
        Some("snapped") => femtovg::GlyphFitting::Snapped,
        Some("fitted") => femtovg::GlyphFitting::Fitted,
        _ => femtovg::GlyphFitting::Subpixel,
    }
}

/// `observer` sees winit events before Slint dispatches them; it changes no dispatch.
pub fn select(software: bool, observer: crate::native_events::Observer) -> Result<(), String> {
    // Read by the renderer when a window creates its canvas, so before any window.
    femtovg::set_glyph_fitting(glyph_fitting(
        std::env::var("MIDNIGHT_GLYPH_FITTING").ok().as_deref(),
    ));
    let explicit_renderer = std::env::var_os("SLINT_BACKEND").is_some();
    let mut selector =
        slint::BackendSelector::new().with_winit_custom_application_handler(observer);
    if software {
        selector = selector
            .backend_name("winit".into())
            .renderer_name("software".into());
    } else if !explicit_renderer {
        selector = selector
            .backend_name("winit".into())
            .renderer_name("femtovg-wgpu".into());
    }
    native::configure(selector, !software && !explicit_renderer)
        .select()
        .map_err(|error| error.to_string())?;
    native::initialize_identity().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::glyph_fitting;
    use femtovg::GlyphFitting;

    #[test]
    fn text_fitting_is_opt_in_by_exact_name() {
        assert_eq!(glyph_fitting(None), GlyphFitting::Subpixel);
        assert_eq!(glyph_fitting(Some("snapped")), GlyphFitting::Snapped);
        assert_eq!(glyph_fitting(Some("fitted")), GlyphFitting::Fitted);
        for other in ["", "Fitted", "fitted ", "subpixel", "1", "on"] {
            assert_eq!(
                glyph_fitting(Some(other)),
                GlyphFitting::Subpixel,
                "{other:?}"
            );
        }
    }
}
