//! Bounded diagnostic timestamps, formatted only after the native event loop ends.
use midnight_viewer_ui::ViewerWindow;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub(super) struct Trace(Rc<RefCell<State>>);
struct State {
    origin: Instant,
    utc_ms: f64,
    enabled: bool,
    stage: usize,
    window: Option<slint::Weak<ViewerWindow>>,
    rows: Vec<Row>,
}
struct Row {
    elapsed_ms: f64,
    kind: &'static str,
    stage: usize,
    position: f32,
    displayed: f32,
    tile_y: f32,
    pending: bool,
    retained: bool,
    presentation: &'static str,
    raster_y: f32,
    raster_uploads: i32,
    tile_uploads: i32,
    extra: f64,
}
impl Trace {
    pub fn new(enabled: bool) -> Self {
        Self(Rc::new(RefCell::new(State {
            origin: Instant::now(),
            utc_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64()
                * 1000.0,
            enabled,
            stage: 0,
            window: None,
            rows: Vec::with_capacity(if enabled { 65536 } else { 0 }),
        })))
    }
    pub fn bind(&self, window: slint::Weak<ViewerWindow>) {
        self.0.borrow_mut().window = Some(window);
    }
    pub fn stage(&self, stage: usize) {
        self.0.borrow_mut().stage = stage;
        self.record("stage", 0.0);
    }
    pub fn record(&self, kind: &'static str, extra: f64) {
        let mut state = self.0.borrow_mut();
        if !state.enabled || state.rows.len() >= 65536 {
            return;
        }
        let window = state.window.as_ref().and_then(slint::Weak::upgrade);
        let row = Row {
            elapsed_ms: state.origin.elapsed().as_secs_f64() * 1000.0,
            kind,
            stage: state.stage,
            position: window
                .as_ref()
                .map_or(f32::NAN, |window| window.get_position()),
            displayed: window
                .as_ref()
                .map_or(f32::NAN, |window| window.get_displayed_position()),
            tile_y: window
                .as_ref()
                .map_or(f32::NAN, |window| window.get_tile_offset_y()),
            pending: window
                .as_ref()
                .is_some_and(|window| window.get_position_pending()),
            retained: window
                .as_ref()
                .is_some_and(|window| window.get_retained_raster()),
            presentation: window.as_ref().map_or("unavailable", super::presentation),
            raster_y: window.as_ref().map_or(f32::NAN, |window| {
                window.get_raster_offset_y() * window.get_content_height()
            }),
            raster_uploads: window
                .as_ref()
                .map_or(0, |window| window.get_raster_upload_count()),
            tile_uploads: window
                .as_ref()
                .map_or(0, |window| window.get_tile_upload_count()),
            extra,
        };
        state.rows.push(row);
    }
    pub fn rows(&self) -> Vec<String> {
        let state = self.0.borrow();
        if !state.enabled {
            return Vec::new();
        }
        std::iter::once(format!("timeline_origin_utc_ms={:.3}", state.utc_ms)).chain(
            state.rows.iter().map(|row| format!(
                "timeline at_ms={:.3} stage={} kind={} position={:.9} displayed={:.9} tile_y={:.6} pending={} retained={} raster_y_logical={:.4} raster_uploads={} tile_uploads={} extra={:.3} presentation={}",
                row.elapsed_ms, row.stage, row.kind, row.position, row.displayed,
                row.tile_y, row.pending, row.retained, row.raster_y, row.raster_uploads, row.tile_uploads, row.extra, row.presentation
            ))
        ).collect()
    }
}
impl Default for Trace {
    fn default() -> Self {
        Self::new(false)
    }
}
