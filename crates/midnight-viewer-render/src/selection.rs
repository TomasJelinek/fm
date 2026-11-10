use crate::{LayoutError, Rect, TextPage};
use parley::{Affinity, Cursor, Selection};
use std::ops::Range;

impl TextPage {
    /// The logical source range and physical top of the visual line at `y`.
    /// Unlike hit testing at x=0 this preserves the logical start of RTL lines.
    pub fn line_at_y(&self, y: f32) -> Result<(Range<usize>, f32), LayoutError> {
        if !y.is_finite() {
            return Err(LayoutError::InvalidGeometry);
        }
        if let Some(grid) = &self.composite {
            return grid.line_at_y(y);
        }
        let line = self
            .layout
            .lines()
            .find(|line| line.metrics().block_max_coord > y)
            .or_else(|| self.layout.lines().last())
            .ok_or(LayoutError::InvalidRange)?;
        let range = line.text_range();
        Ok((
            self.source_map.to_source(range.start)..self.source_map.to_source(range.end),
            line.metrics().block_min_coord,
        ))
    }

    /// Downstream caret geometry at an original UTF-8 boundary, including EOF.
    pub fn caret_geometry(&self, index: usize) -> Result<Rect, LayoutError> {
        self.selected_text(0..index)?;
        if let Some(grid) = &self.composite {
            return grid.caret_geometry(index);
        }
        let cursor = Cursor::from_byte_index(
            &self.layout,
            self.source_map.to_layout(index),
            Affinity::Downstream,
        );
        let rect = cursor.geometry(&self.layout, 1.0);
        Ok(Rect {
            x0: rect.x0,
            y0: rect.y0,
            x1: rect.x1,
            y1: rect.y1,
        })
    }

    /// Hit testing uses physical coordinates in the exact layout used for drawing.
    pub fn hit_test(&self, x: f32, y: f32) -> Result<usize, LayoutError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(LayoutError::InvalidGeometry);
        }
        if let Some(grid) = &self.composite {
            return grid.hit_test(x, y);
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
        if let Some(grid) = &self.composite {
            return grid.selection_geometry(range);
        }
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
