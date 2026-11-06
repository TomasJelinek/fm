use crate::{LayoutError, Rect, TextPage};
use parley::{Affinity, Cursor, Selection};
use std::ops::Range;

impl TextPage {
    /// Hit testing uses physical coordinates in the exact layout used for drawing.
    pub fn hit_test(&self, x: f32, y: f32) -> Result<usize, LayoutError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(LayoutError::InvalidGeometry);
        }
        let index = self
            .source_map
            .to_source(Cursor::from_point(&self.layout, x, y).index());
        self.selected_text(0..index)?;
        Ok(index)
    }

    /// Rectangle positions remain in the layout's physical coordinate space.
    pub fn selection_geometry(&self, range: Range<usize>) -> Result<Vec<Rect>, LayoutError> {
        self.selected_text(range.clone())?;
        let range = self.source_map.to_layout(range.start)..self.source_map.to_layout(range.end);
        let selection = Selection::new(
            Cursor::from_byte_index(&self.layout, range.start, Affinity::Downstream),
            Cursor::from_byte_index(&self.layout, range.end, Affinity::Upstream),
        );
        let mut rectangles = Vec::new();
        selection.geometry_with(&self.layout, |rect, _| {
            rectangles.push(Rect {
                x0: rect.x0,
                y0: rect.y0,
                x1: rect.x1,
                y1: rect.y1,
            });
        });
        Ok(rectangles)
    }

    /// Never reconstruct clipboard text from glyph order or displayed pixels.
    pub fn selected_text(&self, range: Range<usize>) -> Result<&str, LayoutError> {
        self.text.get(range).ok_or(LayoutError::InvalidRange)
    }
}
