//! Source-order row coordinates, independent of a bounded fragment's byte density.
use crate::TextPage;

impl TextPage {
    /// Visual row starts in original UTF-8 coordinates, plus the layout endpoint.
    /// Parallel table cells form one semantic row; their column order is not vertical order.
    pub fn scroll_anchors(&self) -> Vec<(usize, f32)> {
        let mut rows = Vec::new();
        self.append_scroll_anchors(0, 0.0, &mut rows);
        rows
    }

    fn append_scroll_anchors(&self, offset: usize, y: f32, rows: &mut Vec<(usize, f32)>) {
        if let Some(grid) = &self.composite {
            let mut index = 0;
            while let Some(part) = grid.parts.get(index) {
                if let Some(row) = &part.table_row {
                    let end = index
                        + grid.parts[index..]
                            .iter()
                            .take_while(|part| part.table_row.as_ref() == Some(row))
                            .count();
                    let top = grid.parts[index..end]
                        .iter()
                        .map(|part| part.origin[1])
                        .fold(f32::INFINITY, f32::min);
                    let bottom = grid.parts[index..end]
                        .iter()
                        .map(|part| part.origin[1] + part.page.height())
                        .fold(0.0, f32::max);
                    rows.push((offset + row.start, y + top));
                    rows.push((offset + row.end, y + bottom));
                    index = end;
                } else {
                    part.page.append_scroll_anchors(
                        offset + part.range.start,
                        y + part.origin[1],
                        rows,
                    );
                    index += 1;
                }
            }
        } else {
            rows.extend(self.layout.lines().map(|line| {
                (
                    offset + self.source_map.to_source(line.text_range().start),
                    y + line.metrics().block_min_coord,
                )
            }));
        }
        // A semantic endpoint belongs to this span, not its next neighbor. This
        // keeps the final row stable when a refill adds a following paragraph.
        rows.push((offset + self.text.len(), y + self.height()));
    }
}
