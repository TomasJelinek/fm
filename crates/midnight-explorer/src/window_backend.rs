//! Renderer policy shared by startup paths; native window options stay per OS.

#[cfg(target_os = "linux")]
#[path = "window_backend_linux.rs"]
mod native;
#[cfg(windows)]
#[path = "window_backend_windows.rs"]
mod native;

/// How the default renderer fits text to the pixel grid. The explorer uses
/// `fitted`, which puts stems on whole pixel columns. `MIDNIGHT_GLYPH_FITTING`
/// selects another fitting for a run, to compare them or to rule the fitting
/// out: `subpixel` is the renderer's own placement, `snapped` whole-pixel
/// glyphs without horizontal fitting.
fn glyph_fitting(choice: Option<&str>) -> femtovg::GlyphFitting {
    match choice {
        Some("subpixel") => femtovg::GlyphFitting::Subpixel,
        Some("snapped") => femtovg::GlyphFitting::Snapped,
        _ => femtovg::GlyphFitting::Fitted,
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
    fn text_is_fitted_unless_another_fitting_is_named_exactly() {
        assert_eq!(glyph_fitting(None), GlyphFitting::Fitted);
        assert_eq!(glyph_fitting(Some("fitted")), GlyphFitting::Fitted);
        assert_eq!(glyph_fitting(Some("subpixel")), GlyphFitting::Subpixel);
        assert_eq!(glyph_fitting(Some("snapped")), GlyphFitting::Snapped);
        for other in ["", "Subpixel", "snapped ", "off", "0", "none"] {
            assert_eq!(
                glyph_fitting(Some(other)),
                GlyphFitting::Fitted,
                "{other:?}"
            );
        }
    }
}
