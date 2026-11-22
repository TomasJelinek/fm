//! App chrome and document palette are separate from document typography and layout.
use crate::*;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub(super) struct AppAppearance {
    appearance: ViewerAppearance,
    theme: PreferenceTheme,
    density: UiDensity,
    desktop: u16,
    touch: u16,
}

impl AppAppearance {
    pub fn new(
        appearance: &ViewerAppearance,
        theme: PreferenceTheme,
        density: UiDensity,
        desktop: u16,
        touch: u16,
    ) -> Self {
        Self {
            appearance: appearance.clone(),
            theme,
            density,
            desktop,
            touch,
        }
    }

    pub fn present(&self, window: &ViewerWindow) {
        window.invoke_apply_tool_theme(
            self.theme as i32,
            i32::from(self.density == UiDensity::Touch),
            self.appearance.family.as_str().into(),
            i32::from(self.desktop),
            i32::from(self.touch),
        );
        // The caller captures these render colors from the same displayed Theme.
        window.set_foreground(color(self.appearance.foreground));
        window.set_background_color(color(self.appearance.background));
        window.set_accent(color(self.appearance.accent));
    }

    /// Submit this window's document palette, and keep it for activation changes.
    pub fn restore_palette(&self, open: &OpenWindow) {
        *open.app_appearance.borrow_mut() = Some(self.clone());
        let _ = open
            .palette
            .restore_palette(&open.session, &self.document(open));
    }

    /// The application colors, with the theme's dimmer inactive selection pair while
    /// `open` is inactive. That pair is the window's own Theme, which `present` set.
    fn document(&self, open: &OpenWindow) -> ViewerAppearance {
        let mut document = self.appearance.clone();
        if !open.active.get() {
            document.selection = rgba(open.window.get_inactive_selection_background());
            document.selection_text = rgba(open.window.get_inactive_selection_text());
        }
        document
    }

    pub fn merge_palette(&self, document: &mut ViewerAppearance) {
        document.foreground = self.appearance.foreground;
        document.background = self.appearance.background;
        document.selection = self.appearance.selection;
        document.selection_text = self.appearance.selection_text;
        document.accent = self.appearance.accent;
    }
}

/// The window was activated or deactivated: its selection is painted again with the
/// theme's active or inactive pair. Text, page and accent colors stay.
pub(super) fn activation_changed(open: &OpenWindow, active: bool) {
    if open.active.replace(active) == active {
        return;
    }
    let app = open.app_appearance.borrow().clone();
    if let Some(app) = app {
        app.restore_palette(open);
    }
}

fn rgba(color: slint::Color) -> [u8; 4] {
    let color = color.to_argb_u8();
    [color.red, color.green, color.blue, color.alpha]
}

impl PartialEq for AppAppearance {
    fn eq(&self, other: &Self) -> bool {
        self.theme == other.theme
            && self.density == other.density
            && self.desktop == other.desktop
            && self.touch == other.touch
            && self.appearance.family == other.appearance.family
            && self.appearance.font_size.to_bits() == other.appearance.font_size.to_bits()
            && Palette::from(&self.appearance) == Palette::from(&other.appearance)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Palette([[u8; 4]; 5]);

impl From<&ViewerAppearance> for Palette {
    fn from(value: &ViewerAppearance) -> Self {
        Self([
            value.foreground,
            value.background,
            value.selection,
            value.selection_text,
            value.accent,
        ])
    }
}

#[derive(Default)]
struct PaletteState {
    requested: Option<(u64, Palette)>,
    applied: Option<(u64, Option<u64>)>,
}

/// Only scalar fences cross to pixel preparation. Existing displayed buffers stay owned
/// by the normal presentation/retirement paths throughout a palette request.
#[derive(Clone, Default)]
pub(super) struct PaletteGate(Arc<Mutex<PaletteState>>);

impl PaletteGate {
    /// Seed only the accepted configuration; a real generation-bound frame fence
    /// is still required. The caller has not published this session to writers.
    pub fn opening(session: &WindowSession, appearance: &ViewerAppearance) -> Self {
        Self(Arc::new(Mutex::new(PaletteState {
            requested: Some((session.palette_revision(), Palette::from(appearance))),
            applied: None,
        })))
    }

    fn state(&self) -> MutexGuard<'_, PaletteState> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn restore_palette(
        &self,
        session: &WindowSession,
        appearance: &ViewerAppearance,
    ) -> Result<(), String> {
        let mut state = self.state();
        let palette = Palette::from(appearance);
        if state
            .requested
            .is_some_and(|(_, previous)| previous == palette)
        {
            return Ok(());
        }
        // Serialize this bounded, scalar admission with the preparation worker's
        // observe call: even an immediate acknowledgement must see its request.
        // Failure preserves the prior fence and never replaces document status.
        let revision = session.restore_palette(appearance)?;
        state.requested = Some((revision, palette));
        state.applied = None;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn accepted(&self, appearance: &ViewerAppearance, revision: u64) {
        let mut state = self.state();
        let request = (revision, Palette::from(appearance));
        if state.requested != Some(request) {
            state.requested = Some(request);
            state.applied = None;
        }
    }

    pub fn observe(&self, outputs: &[ViewerOutput]) {
        if outputs
            .iter()
            .any(|output| matches!(output, ViewerOutput::Closed))
        {
            return;
        }
        let mut state = self.state();
        for output in outputs {
            if let ViewerOutput::PaletteApplied {
                revision,
                generation,
                minimum_frame_revision,
            } = output
                && state
                    .requested
                    .is_some_and(|(requested, _)| requested == *revision)
                && state
                    .applied
                    .is_none_or(|(previous, _)| *generation >= previous)
            {
                let minimum = match state.applied {
                    Some((previous, Some(floor))) if previous == *generation => {
                        Some(minimum_frame_revision.map_or(floor, |next| floor.max(next)))
                    }
                    _ => *minimum_frame_revision,
                };
                state.applied = Some((*generation, minimum));
            }
        }
    }

    pub fn allows(&self, frame: &ViewerFrame) -> bool {
        // Browser content and original image/PDF/video pixels own their colors.
        if frame.image.is_some()
            || matches!(
                frame.mode,
                VIEWER_MODE_IMAGE | VIEWER_MODE_HTML | VIEWER_MODE_VIDEO
            )
        {
            return true;
        }
        let state = self.state();
        state.applied.is_some_and(|(generation, floor)| {
            generation == frame.generation && floor.is_some_and(|minimum| frame.revision >= minimum)
        })
    }

    #[cfg(test)]
    pub(super) fn requested_revision(&self) -> Option<u64> {
        self.state().requested.map(|(revision, _)| revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn app() -> ViewerAppearance {
        ViewerAppearance {
            family: "serif".into(),
            font_size: 23.0,
            foreground: [48, 48, 48, 255],
            background: [255; 4],
            selection: [204, 204, 204, 255],
            selection_text: [0; 4],
            accent: [69, 69, 69, 255],
        }
    }

    fn text(revision: u64, generation: u64) -> Box<ViewerFrame> {
        let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(revision, 0, 0.2)
        else {
            unreachable!()
        };
        frame.generation = generation;
        frame
    }

    fn applied(revision: u64, generation: u64, floor: Option<u64>) -> ViewerOutput {
        ViewerOutput::PaletteApplied {
            revision,
            generation,
            minimum_frame_revision: floor,
        }
    }

    fn retained(revision: u64, generation: u64) -> Box<ViewerFrame> {
        let mut frame = text(revision, generation);
        frame.width = 6;
        frame.height = 6;
        frame.pixels = vec![255; 6 * 6 * 4].into();
        frame.raster = Some(ViewerRasterInfo {
            surface: ViewerSurfaceId(77),
            pixel_id: 1,
            origin: [0.0; 2],
            raster_origin: [0.0; 2],
            viewport: [2; 2],
            coverage_min: [0.0; 2],
            coverage_max: [4.0; 2],
            limit_min: [Some(0.0); 2],
            limit_max: [Some(4.0); 2],
            pan_sequence: 0,
            scroll_per_pixel: [0.0; 2],
            scroll_map: None,
            settled: true,
            gutter: None,
            bands: None,
        });
        frame
    }

    #[test]
    fn an_inactive_window_paints_its_selection_with_the_theme_inactive_pair() {
        let (_runtime, manager, open, _service) = image_presentation_tests::open();
        open.timer.stop();
        let mut app = image_presentation_tests::opening_appearance();
        app.selection = [38, 79, 120, 255];
        app.selection_text = [255; 4];
        let present = |theme| {
            manager.apply_appearance(&app, theme, UiDensity::Desktop, 12, 15);
        };
        let requested = || open.palette.state().requested.unwrap();
        present(PreferenceTheme::SlateTeal);
        let (active, Palette(colors)) = requested();
        assert_eq!(colors, Palette::from(&app).0);
        activation_changed(&open, false);
        let (inactive, Palette(colors)) = requested();
        assert_ne!(inactive, active, "a new palette request");
        let pair = || {
            [
                rgba(open.window.get_inactive_selection_background()),
                rgba(open.window.get_inactive_selection_text()),
            ]
        };
        assert_eq!(pair(), [[0x31, 0x40, 0x3c, 255], [255; 4]]);
        assert_eq!(colors[2..4], pair(), "the theme's inactive pair");
        let unchanged = [colors[0], colors[1], colors[4]];
        assert_eq!(unchanged, [app.foreground, app.background, app.accent]);
        // Another deactivation is no change; a new theme keeps the window inactive.
        activation_changed(&open, false);
        assert_eq!(requested().0, inactive);
        present(PreferenceTheme::Light);
        assert_eq!(pair()[0], [0xe5, 0xeb, 0xf1, 255]);
        assert_eq!(requested().1.0[2..4], pair());
        activation_changed(&open, true);
        assert_eq!(requested().1.0, Palette::from(&app).0);
        manager.close_all();
    }

    #[test]
    fn opening_getter_seeds_pending_fence_and_keeps_real_required_input_rejection() {
        let (_runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        let token = open.session.palette_revision();
        assert_eq!(open.palette.requested_revision(), Some(token));
        let preferences = open.session.preferences();
        let shown = open.frame.get();
        let candidate = text(shown + 1, 7);
        assert!(
            !open.palette.allows(&candidate),
            "a getter is not a paint acknowledgement"
        );
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(candidate)],
        );
        assert_eq!(open.frame.get(), shown);
        assert!(open.window.get_tiled());
        assert_eq!(open.session.palette_revision(), token);
        assert!(Arc::ptr_eq(&preferences, &open.session.preferences()));
        open.session.close();
        assert_eq!(
            open.session.palette_revision(),
            token,
            "token remains readable after close"
        );
        let appearance = image_presentation_tests::opening_appearance();
        let closed_fence = PaletteGate::opening(&open.session, &appearance);
        assert_eq!(closed_fence.requested_revision(), Some(token));
        assert!(!closed_fence.allows(&text(shown + 1, 7)));
        let error = match prepare_session(&open.session, &appearance) {
            Ok(_) => panic!("closed Session admitted the required RetainedRaster input"),
            Err(error) => error,
        };
        assert_eq!(error, "Viewer input is unavailable.");
        assert_eq!(open.session.palette_revision(), token);
        manager.close_all();
    }

    #[test]
    fn palette_fences_pending_split_drains_rollback_and_exact_positions_generations() {
        // This scalar-only gate fixture covers a valid zero token; it opens no Session.
        let gate = PaletteGate::default();
        let a = app();
        let mut b = a.clone();
        b.background = [23, 28, 42, 255];
        assert!(
            !gate.allows(&text(10, 7)),
            "startup needs actual palette admission"
        );
        gate.accepted(&a, 0);
        assert!(
            !gate.allows(&text(10, 7)),
            "acceptance precedes acknowledgement"
        );
        gate.observe(&[applied(0, 7, Some(10))]);
        assert!(
            gate.allows(&text(10, 7)),
            "a later drain uses the saved fence"
        );
        gate.accepted(&b, 1);
        gate.accepted(&a, 2); // B was accepted, but never painted.
        gate.observe(&[applied(3, 99, Some(1)), applied(1, 7, Some(11))]);
        assert!(!gate.allows(&text(11, 7)), "old B cannot clear pending A");
        gate.observe(&[applied(2, 7, Some(12))]);
        gate.observe(&[applied(2, 7, Some(1))]);
        assert!(!gate.allows(&text(11, 7)));
        assert!(gate.allows(&text(12, 7)));
        assert!(
            !gate.allows(&text(13, 8)),
            "a positions generation needs its own fence"
        );
        gate.observe(&[applied(2, 8, Some(13))]);
        gate.observe(&[applied(2, 7, Some(1))]);
        assert!(gate.allows(&text(13, 8)));
        assert!(!gate.allows(&text(20, 7)));
        gate.accepted(&a, 2); // An unchanged request must not strand accepted pixels.
        assert!(gate.allows(&text(13, 8)));
        gate.observe(&[applied(2, 9, None)]);
        assert!(
            !gate.allows(&text(14, 9)),
            "None does not certify app-painted text"
        );
        let mut image = image_presentation_tests::frame();
        image.generation = 9;
        assert!(gate.allows(&image));
        image.image = None;
        assert!(gate.allows(&image));
        let mut browser = text(14, 9);
        browser.mode = VIEWER_MODE_HTML;
        assert!(gate.allows(&browser));
        for mode in [1, 2, 3, 4] {
            let mut frame = text(14, 9);
            frame.mode = mode;
            assert!(
                !gate.allows(&frame),
                "mode {mode} requires app-painted pixels"
            );
        }
    }

    #[test]
    fn app_theme_primes_preview_windows_and_rollback_preserves_document_edits() {
        let (_runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        let saves = Rc::new(Cell::new(0));
        let saved = saves.clone();
        manager.set_preferences_listener(Rc::new(move |_| saved.set(saved.get() + 1)));
        let document = open.session.preferences();
        let before = (
            open.layout.borrow().token,
            open.frame_generation.get(),
            open.frame.get(),
        );
        let appearance = app();
        manager.apply_appearance(
            &appearance,
            PreferenceTheme::Light,
            UiDensity::Desktop,
            23,
            29,
        );
        assert_eq!(open.window.get_ui_font_family(), "serif");
        assert_eq!(open.window.get_ui_font_size(), 23.0);
        assert_eq!(
            open.window.get_background_color(),
            color(appearance.background)
        );
        assert!(Arc::ptr_eq(&document, &open.session.preferences()));
        assert_eq!(
            before,
            (
                open.layout.borrow().token,
                open.frame_generation.get(),
                open.frame.get()
            )
        );
        let requested = open.palette.requested_revision();
        manager.apply_appearance(
            &appearance,
            PreferenceTheme::Light,
            UiDensity::Desktop,
            23,
            29,
        );
        assert_eq!(open.palette.requested_revision(), requested);
        let mut preview = appearance.clone();
        preview.family = "sans-serif".into();
        preview.foreground = [222, 222, 222, 255];
        preview.background = [32, 32, 32, 255];
        preview.selection = [85, 85, 85, 255];
        preview.accent = [212, 212, 212, 255];
        manager.apply_appearance(&preview, PreferenceTheme::Dark, UiDensity::Touch, 23, 29);
        let path = std::env::temp_dir().join("appearance-late-viewer.txt");
        let entries: Arc<[Entry]> =
            vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
        let late = manager
            .open(
                ViewerLaunch {
                    find: None,
                    associations: Default::default(),
                    tree: None,
                    generated: None,
                    preferences: document.clone(),
                    archive: None,
                    view: Arc::new(DirectoryIndex::from_entries(&entries)),
                    entries,
                    focused: path,
                    basic: false,
                    disabled_plugins: Arc::from([]),
                },
                ViewerAppearance {
                    family: "monospace".into(),
                    font_size: 9.0,
                    ..appearance.clone()
                },
                slint::LogicalSize::new(900.0, 600.0),
                Rc::new(|_, _| None),
                Rc::new(|_| {}),
            )
            .unwrap()
            .upgrade()
            .unwrap();
        assert_eq!(late.get_ui_font_size(), 29.0);
        assert_eq!(late.get_ui_font_family(), "sans-serif");
        assert_eq!(saves.get(), 0);
        image_presentation_tests::present(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(text(10, 7))],
        );
        open.window.invoke_change_font_size(31);
        let edited = open.session.preferences();
        assert_eq!(edited.text_font.size, 31);
        manager.apply_appearance(
            &appearance,
            PreferenceTheme::Light,
            UiDensity::Desktop,
            23,
            29,
        );
        assert_eq!(late.get_ui_font_size(), 23.0);
        assert_eq!(late.get_ui_font_family(), "serif");
        assert_eq!(late.get_background_color(), color(appearance.background));
        assert!(Arc::ptr_eq(&edited, &open.session.preferences()));
        assert_eq!(open.window.get_viewer_font_size(), 31);
        assert_eq!(
            saves.get(),
            1,
            "only the explicit document edit reports a preference"
        );
        let requested = open.palette.requested_revision();
        let status = open.window.get_status();
        open.session.close();
        manager.apply_appearance(&preview, PreferenceTheme::Dark, UiDensity::Touch, 23, 29);
        assert_eq!(
            open.palette.requested_revision(),
            requested,
            "refusal preserves its fence"
        );
        assert_eq!(open.window.get_status(), status);
        manager.close_all();
    }

    #[test]
    fn delayed_pixels_and_retained_metadata_cannot_replace_a_pending_or_newer_palette() {
        let (_runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        let appearance = image_presentation_tests::opening_appearance();
        let mut dark = appearance.clone();
        dark.background = [32, 32, 32, 255];
        let deliver = |frame: Box<ViewerFrame>, pixels| {
            pixel_delivery::deliver_batch(
                &open,
                &service,
                &|_| {},
                pixel_preparation::PreparedBatch {
                    outputs: vec![ViewerOutput::Frame(frame)],
                    pixels,
                },
            );
        };
        let initial = open.session.palette_revision();
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![applied(initial, 7, Some(10))],
        );
        let accepted = text(10, 7);
        let pixels = pixel_preparation::prepare(&accepted);
        deliver(accepted, pixels);
        assert_eq!(open.frame.get(), 10);
        let visible = open.window.get_surface();
        let old = text(11, 7);
        let delayed = pixel_preparation::prepare(&old);
        open.palette
            .restore_palette(&open.session, &dark)
            .inspect_err(|error| {
                eprintln!("palette restoration refused: {error}");
                eprintln!(
                    "backend open result: {:?}",
                    open.session.mailbox.take_open_result()
                );
                eprintln!("backend status outputs: {:?}", open.session.mailbox.take());
            })
            .unwrap();
        let changed = open.session.palette_revision();
        assert_ne!(changed, initial);
        deliver(old, delayed);
        assert_eq!(open.frame.get(), 10);
        assert_eq!(open.window.get_surface(), visible);
        let status = open.window.get_status();
        deliver(text(12, 7), Err("obsolete conversion failure".into()));
        assert_eq!(open.window.get_status(), status);
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![applied(initial, 7, Some(11))],
        );
        assert_eq!(open.frame.get(), 10);
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![applied(changed, 7, Some(13))],
        );
        let older = text(12, 7);
        let pixels = pixel_preparation::prepare(&older);
        deliver(older, pixels);
        assert_eq!(
            open.frame.get(),
            10,
            "acknowledged floor still rejects older pixels"
        );
        let mut recolored = text(13, 7);
        recolored.pixels = [19, 37, 59, 255].repeat(4).into();
        let pixels = pixel_preparation::prepare(&recolored);
        deliver(recolored, pixels);
        assert_eq!(open.frame.get(), 13);
        let (size, pixels) = image_presentation_tests::bitmap();
        let x = open.window.get_content_left() as usize + 2;
        let y = open.window.get_content_top() as usize + 2;
        let pixel = pixels[y * size.width as usize + x];
        assert_eq!([pixel.r, pixel.g, pixel.b], [19, 37, 59]);
        let mut retained = retained(14, 7);
        assert!(raster_presentation::receive(&open, &retained));
        let uploads = open.window.get_raster_upload_count();
        let position = open.window.get_raster_offset_y();
        open.palette
            .restore_palette(&open.session, &appearance)
            .unwrap();
        assert_ne!(open.session.palette_revision(), changed);
        retained.revision = 15;
        retained.raster.as_mut().unwrap().origin = [0.0, 1.0];
        assert!(!raster_presentation::receive(&open, &retained));
        assert_eq!(open.window.get_raster_upload_count(), uploads);
        assert_eq!(open.window.get_raster_offset_y(), position);
        manager.close_all();
    }

    #[test]
    fn newer_layout_rejects_palette_valid_failed_pixels_before_delivery_side_effects() {
        let (_runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        let inputs = Rc::new(RefCell::new(Vec::new()));
        let captured = inputs.clone();
        *open.input.borrow_mut() = Rc::new(move |input| {
            captured.borrow_mut().push(input);
            true
        });
        let revision = open.palette.requested_revision().unwrap();
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![applied(revision, 7, Some(10))],
        );
        let mut shown = retained(10, 7);
        shown.raster.as_mut().unwrap().origin = [2.0, 1.0];
        let pixels = pixel_preparation::prepare(&shown);
        pixel_delivery::deliver_batch(
            &open,
            &service,
            &|_| {},
            pixel_preparation::PreparedBatch {
                outputs: vec![ViewerOutput::Frame(shown)],
                pixels,
            },
        );
        assert_eq!(open.frame.get(), 10);
        let lease = || {
            let stored = open.prepared_pixels.borrow();
            let stored = stored.as_ref().unwrap();
            (
                stored.frame_revision,
                stored.body.as_bytes().as_ptr() as usize,
            )
        };
        let before_lease = lease();
        let before = (
            open.window.get_status(),
            open.window.get_surface(),
            open.retained_raster.borrow().camera.origin,
            (
                open.window.get_raster_offset_x(),
                open.window.get_raster_offset_y(),
            ),
        );
        let mut stale = text(11, 7);
        stale.pixels = Arc::from([]);
        assert!(
            open.palette.allows(&stale),
            "this is a layout fence, not a palette rejection"
        );
        let pixels = pixel_preparation::prepare(&stale);
        assert!(pixels.is_err(), "exercise the actual malformed-pixel error");
        open.text_position.borrow_mut().request(false, 0.7).unwrap();
        pixel_delivery::deliver_batch(
            &open,
            &service,
            &|_| {},
            pixel_preparation::PreparedBatch {
                outputs: vec![ViewerOutput::Frame(stale.clone())],
                pixels: pixels.clone(),
            },
        );
        assert_eq!(open.window.get_status(), before.0);
        assert_eq!(lease(), before_lease);
        assert_eq!(open.frame.get(), 10);
        assert!(
            open.text_position.borrow().has_pending(),
            "stale sequence cannot acknowledge the requested position"
        );
        pixel_delivery::deliver_batch(
            &open,
            &service,
            &|_| {},
            pixel_preparation::PreparedBatch {
                outputs: vec![
                    ViewerOutput::Frame(stale),
                    ViewerOutput::LayoutSelected {
                        token: 2,
                        mode: VIEWER_MODE_HTML,
                        content_generation: 8,
                    },
                ],
                pixels,
            },
        );
        assert_eq!(open.window.get_status(), before.0);
        assert_eq!(open.window.get_surface(), before.1);
        assert_eq!(open.retained_raster.borrow().camera.origin, before.2);
        assert_eq!(
            (
                open.window.get_raster_offset_x(),
                open.window.get_raster_offset_y()
            ),
            before.3
        );
        assert_eq!(lease(), before_lease);
        assert_eq!((open.frame.get(), open.frame_generation.get()), (10, 7));
        let layout = open.layout.borrow();
        assert_eq!(
            (layout.token, layout.mode, layout.generation_floor),
            (2, VIEWER_MODE_HTML, 8)
        );
        assert!(!layout.pending_ack && layout.awaiting_frame);
        drop(layout);
        assert!(open.window.get_web_mode());
        assert_eq!(open.window.get_position_context(), "layout:2:8");
        present_outputs(&open, &service, &|_| {}, vec![]);
        assert!(
            matches!(
                inputs.borrow().as_slice(),
                [ViewerInput::LayoutReady { token: 2, .. }]
            ),
            "one control application/acknowledgement"
        );
        manager.close_all();
    }
}
