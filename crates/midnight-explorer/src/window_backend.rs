//! Renderer policy shared by startup paths; native window options stay per OS.

#[cfg(target_os = "linux")]
#[path = "window_backend_linux.rs"]
mod native;
#[cfg(windows)]
#[path = "window_backend_windows.rs"]
mod native;

/// How the default renderer fits text to the pixel grid: the Options choice,
/// which applies from the next start. `MIDNIGHT_GLYPH_FITTING` (`fitted`,
/// `snapped` or `subpixel`) overrides it for a run, to compare them or to rule
/// the fitting out; any other value is ignored.
fn glyph_fitting(
    choice: Option<&str>,
    stored: midnight_core::GlyphFitting,
) -> femtovg::GlyphFitting {
    match choice
        .and_then(midnight_core::GlyphFitting::from_name)
        .unwrap_or(stored)
    {
        midnight_core::GlyphFitting::Fitted => femtovg::GlyphFitting::Fitted,
        midnight_core::GlyphFitting::Snapped => femtovg::GlyphFitting::Snapped,
        midnight_core::GlyphFitting::Subpixel => femtovg::GlyphFitting::Subpixel,
    }
}

/// `observer` sees winit events before Slint dispatches them; it changes no dispatch.
pub fn select(
    software: bool,
    observer: crate::native_events::Observer,
    stored_fitting: midnight_core::GlyphFitting,
) -> Result<(), String> {
    // Read by the renderer when a window creates its canvas, so before any window.
    femtovg::set_glyph_fitting(glyph_fitting(
        std::env::var("MIDNIGHT_GLYPH_FITTING").ok().as_deref(),
        stored_fitting,
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
    use midnight_core::GlyphFitting as Stored;

    #[test]
    fn the_stored_choice_applies_unless_the_environment_names_another_exactly() {
        assert_eq!(glyph_fitting(None, Stored::Fitted), GlyphFitting::Fitted);
        assert_eq!(glyph_fitting(None, Stored::Snapped), GlyphFitting::Snapped);
        assert_eq!(
            glyph_fitting(None, Stored::Subpixel),
            GlyphFitting::Subpixel
        );
        assert_eq!(
            glyph_fitting(Some("subpixel"), Stored::Fitted),
            GlyphFitting::Subpixel
        );
        assert_eq!(
            glyph_fitting(Some("fitted"), Stored::Snapped),
            GlyphFitting::Fitted
        );
        for other in ["", "Subpixel", "snapped ", "off", "0", "none"] {
            assert_eq!(
                glyph_fitting(Some(other), Stored::Snapped),
                GlyphFitting::Snapped,
                "{other:?}"
            );
        }
    }
}
