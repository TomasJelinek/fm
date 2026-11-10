use crate::{ColorSpan, LayoutError, Rect, StyleSpan, TextOptions, TextPage, TextRenderer};
use parley::{Alignment, AlignmentOptions};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone)]
pub struct TableCell {
    pub range: Range<usize>,
    pub alignment: CellAlignment,
}

/// Table row ranges and cell ranges refer to the original decoded page text.
#[derive(Clone)]
pub struct TableRow {
    pub range: Range<usize>,
    pub cells: Vec<TableCell>,
    pub header: bool,
}

pub(crate) struct PlacedPage {
    pub range: Range<usize>,
    pub table_row: Option<Range<usize>>,
    pub origin: [f32; 2],
    pub width: f32,
    pub page: TextPage,
}

pub(crate) struct Composite {
    pub parts: Vec<PlacedPage>,
    pub backgrounds: Vec<crate::raster::Fill>,
    pub borders: Vec<Rect>,
    pub border_color: [u8; 4],
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct TablePresentation {
    pub border: [u8; 4],
    pub alternate: [u8; 4],
}

impl TextRenderer {
    pub fn layout_with_tables(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
        styles: &[StyleSpan],
        tables: &[TableRow],
    ) -> Result<TextPage, LayoutError> {
        self.layout_tables(text, options, spans, styles, tables, None)
    }

    pub(crate) fn layout_tables(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
        styles: &[StyleSpan],
        tables: &[TableRow],
        presentation: Option<TablePresentation>,
    ) -> Result<TextPage, LayoutError> {
        if tables.is_empty() {
            return self.layout_with_styles(text, options, spans, styles);
        }
        validate_tables(text, options, spans, styles, tables)?;
        let mut result = self.layout_with_styles("", options, &[], &[])?;
        let mut grid = Composite {
            parts: Vec::new(),
            backgrounds: Vec::new(),
            borders: Vec::new(),
            border_color: presentation.map_or(
                [
                    options.foreground[0],
                    options.foreground[1],
                    options.foreground[2],
                    100,
                ],
                |style| style.border,
            ),
            width: 0.0,
            height: 0.0,
        };
        let mut cursor = 0;
        let mut rows = tables.iter().peekable();
        while let Some(row) = rows.next() {
            if cursor < row.range.start {
                self.place_prose(
                    text,
                    cursor..row.range.start,
                    options,
                    spans,
                    styles,
                    &mut grid,
                )?;
            }
            let columns = row.cells.len();
            let mut group = vec![row];
            let mut end = row.range.end;
            while rows.peek().is_some_and(|next| {
                !next.header
                    && next.cells.len() == columns
                    && text[end..next.range.start].trim().is_empty()
            }) {
                let next = rows.next().ok_or(LayoutError::InvalidRange)?;
                end = next.range.end;
                group.push(next);
            }
            let padding_x = if presentation.is_some() {
                13.0 * options.scale
            } else {
                options.size * options.scale * 0.45
            };
            let padding_y = if presentation.is_some() {
                6.0 * options.scale
            } else {
                padding_x
            };
            let minimum = (options.size * options.scale * 5.5).max(48.0);
            let available = options
                .wrap_width
                .unwrap_or(720.0 * options.scale)
                .max(minimum * columns as f32);
            let column_widths = if presentation.is_some() {
                self.intrinsic_columns(text, options, styles, &group, padding_x)?
            } else {
                vec![available / columns as f32; columns]
            };
            let shared_border = if presentation.is_some() {
                options.scale
            } else {
                0.0
            };
            let width = column_widths.iter().sum::<f32>() + shared_border;
            let mut body_row = 0;
            for current in group {
                let top = grid.height;
                let first_part = grid.parts.len();
                let mut height = options.size * options.scale * 1.3;
                let mut x: f32 = 0.0;
                for (column, cell) in current.cells.iter().enumerate() {
                    let column_width = column_widths[column];
                    let header_weight = if presentation.is_some() { 600 } else { 700 };
                    let cell_options = TextOptions {
                        wrap_width: Some(column_width - padding_x * 2.0 - shared_border),
                        tabular_numbers: presentation.is_some() || options.tabular_numbers,
                        weight: if current.header {
                            header_weight
                        } else {
                            options.weight
                        },
                        ..*options
                    };
                    let cell_styles = table_styles(
                        styles,
                        &cell.range,
                        current.header,
                        presentation.is_some(),
                        options.size,
                    );
                    let mut page = self.layout_with_styles(
                        &text[cell.range.clone()],
                        &cell_options,
                        &clipped_colors(spans, &cell.range),
                        &cell_styles,
                    )?;
                    page.layout.align(
                        match cell.alignment {
                            CellAlignment::Left => Alignment::Left,
                            CellAlignment::Center => Alignment::Center,
                            CellAlignment::Right => Alignment::Right,
                        },
                        AlignmentOptions::default(),
                    );
                    height = height.max(page.height());
                    grid.parts.push(PlacedPage {
                        range: cell.range.clone(),
                        table_row: Some(current.range.clone()),
                        origin: [
                            x + padding_x + shared_border,
                            top + padding_y + shared_border,
                        ],
                        width: column_width - padding_x * 2.0 - shared_border,
                        page,
                    });
                    x += column_width;
                }
                if presentation.is_some() {
                    for part in &mut grid.parts[first_part..] {
                        part.origin[1] += (height - part.page.height()) / 2.0;
                    }
                }
                height += padding_y * 2.0 + shared_border;
                let stroke = if presentation.is_some() {
                    options.scale
                } else {
                    1.0
                };
                grid.borders.push(rect(0.0, top, width, top + stroke));
                let bottom = top + height - stroke + shared_border;
                grid.borders.push(rect(0.0, bottom, width, bottom + stroke));
                let mut x: f32 = 0.0;
                for column_width in column_widths.iter().chain(std::iter::once(&0.0)) {
                    let line_x = x.min(width - stroke);
                    // Content widths may be fractional after compression. Snap
                    // a GitHub border to a physical pixel so conservative raster
                    // bounds do not turn its one-pixel stroke into two pixels.
                    let line_x = if presentation.is_some() {
                        line_x.round()
                    } else {
                        line_x
                    };
                    grid.borders.push(rect(
                        line_x,
                        top,
                        line_x + stroke,
                        top + height + shared_border,
                    ));
                    x += column_width;
                }
                if !current.header
                    && body_row % 2 == 1
                    && let Some(style) = presentation
                {
                    grid.backgrounds.push(crate::raster::Fill {
                        rect: rect(0.0, top, width, top + height),
                        color: style.alternate,
                        radius: 0.0,
                    });
                }
                if !current.header {
                    body_row += 1;
                }
                grid.height += height;
                grid.width = grid.width.max(width);
                cursor = current.range.end;
            }
            grid.height += shared_border;
        }
        if cursor < text.len() {
            self.place_prose(text, cursor..text.len(), options, spans, styles, &mut grid)?;
        }
        result.text = text.into();
        result.composite = Some(grid);
        Ok(result)
    }

    fn intrinsic_columns(
        &mut self,
        text: &str,
        options: &TextOptions<'_>,
        styles: &[StyleSpan],
        rows: &[&TableRow],
        padding: f32,
    ) -> Result<Vec<f32>, LayoutError> {
        let inset = padding * 2.0 + options.scale;
        let mut minimums = vec![inset; rows[0].cells.len()];
        let mut widths = minimums.clone();
        for row in rows {
            for (column, cell) in row.cells.iter().enumerate() {
                let page = self.layout_with_styles(
                    &text[cell.range.clone()],
                    &TextOptions {
                        wrap_width: None,
                        tabular_numbers: true,
                        weight: if row.header { 600 } else { options.weight },
                        ..*options
                    },
                    &[],
                    &table_styles(styles, &cell.range, row.header, true, options.size),
                )?;
                // Measure Unicode line-break opportunities, not a fixed number
                // of characters. Numeric values and other unbreakable tokens
                // determine the minimum; wider phrases can still wrap naturally.
                let content = page.layout.calculate_content_widths();
                minimums[column] =
                    minimums[column].max(content.min.ceil().max(options.scale) + inset);
                widths[column] = widths[column].max(content.max.ceil().max(options.scale) + inset);
            }
        }
        let minimum = minimums.iter().sum::<f32>();
        let available =
            (options.wrap_width.unwrap_or(720.0 * options.scale) - options.scale).max(minimum);
        let total = widths.iter().sum::<f32>();
        if total > available {
            let flexible = total - minimum;
            let target = available - minimum;
            for (width, minimum) in widths.iter_mut().zip(minimums) {
                *width = minimum + (*width - minimum) * target / flexible;
            }
        }
        Ok(widths)
    }

    fn place_prose(
        &mut self,
        text: &str,
        range: Range<usize>,
        options: &TextOptions<'_>,
        spans: &[ColorSpan],
        styles: &[StyleSpan],
        grid: &mut Composite,
    ) -> Result<(), LayoutError> {
        let page = self.layout_with_styles(
            &text[range.clone()],
            options,
            &clipped_colors(spans, &range),
            &clipped_styles(styles, &range),
        )?;
        grid.width = grid.width.max(page.width());
        let top = grid.height;
        grid.height += page.height();
        grid.parts.push(PlacedPage {
            range,
            table_row: None,
            origin: [0.0, top],
            width: page.width(),
            page,
        });
        Ok(())
    }
}

fn table_styles(
    styles: &[StyleSpan],
    range: &Range<usize>,
    header: bool,
    github: bool,
    size: f32,
) -> Vec<StyleSpan> {
    let mut styles = clipped_styles(styles, range);
    for style in &mut styles {
        if header {
            style.weight = style.weight.max(if github { 600 } else { 700 });
        }
        if github && style.monospace {
            style.size = size * 0.85;
        }
    }
    styles
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect {
        x0: x0.into(),
        y0: y0.into(),
        x1: x1.into(),
        y1: y1.into(),
    }
}

pub(crate) fn clipped_colors(spans: &[ColorSpan], range: &Range<usize>) -> Vec<ColorSpan> {
    spans
        .iter()
        .filter_map(|span| {
            intersection(&span.range, range).map(|local| ColorSpan {
                range: local,
                color: span.color,
            })
        })
        .collect()
}

pub(crate) fn clipped_styles(styles: &[StyleSpan], range: &Range<usize>) -> Vec<StyleSpan> {
    styles
        .iter()
        .filter_map(|style| {
            intersection(&style.range, range).map(|local| StyleSpan {
                range: local,
                ..style.clone()
            })
        })
        .collect()
}

fn intersection(span: &Range<usize>, range: &Range<usize>) -> Option<Range<usize>> {
    let start = span.start.max(range.start);
    let end = span.end.min(range.end);
    (start < end).then(|| start - range.start..end - range.start)
}

pub(crate) fn validate_tables(
    text: &str,
    options: &TextOptions<'_>,
    spans: &[ColorSpan],
    styles: &[StyleSpan],
    rows: &[TableRow],
) -> Result<(), LayoutError> {
    crate::validate_input(text, options.size, options.scale, spans)?;
    crate::validate_styles(text, options.scale, styles)?;
    if rows.len() > 512 {
        return Err(LayoutError::LimitExceeded);
    }
    let mut previous = 0;
    let mut cells = 0;
    for row in rows {
        if row.range.start < previous || text.get(row.range.clone()).is_none() {
            return Err(LayoutError::InvalidRange);
        }
        if !(1..=64).contains(&row.cells.len()) {
            return Err(LayoutError::LimitExceeded);
        }
        cells += row.cells.len();
        if cells > 4096 {
            return Err(LayoutError::LimitExceeded);
        }
        let mut end = row.range.start;
        for cell in &row.cells {
            if cell.range.start < end
                || cell.range.end > row.range.end
                || text.get(cell.range.clone()).is_none()
            {
                return Err(LayoutError::InvalidRange);
            }
            end = cell.range.end;
        }
        previous = row.range.end;
    }
    Ok(())
}

impl Composite {
    fn part_at_index(&self, index: usize) -> Result<&PlacedPage, LayoutError> {
        self.parts
            .iter()
            .find(|part| part.range.contains(&index))
            .or_else(|| {
                self.parts
                    .iter()
                    .rev()
                    .find(|part| part.range.start <= index)
            })
            .or_else(|| self.parts.first())
            .ok_or(LayoutError::InvalidRange)
    }

    pub fn caret_geometry(&self, index: usize) -> Result<Rect, LayoutError> {
        let part = self.part_at_index(index)?;
        let local = index.clamp(part.range.start, part.range.end) - part.range.start;
        Ok(translate(part.page.caret_geometry(local)?, part.origin))
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Result<usize, LayoutError> {
        let distance = |part: &PlacedPage| {
            let dx = (part.origin[0] - x)
                .max(0.0)
                .max(x - part.origin[0] - part.width);
            let dy = (part.origin[1] - y)
                .max(0.0)
                .max(y - part.origin[1] - part.page.height());
            dx * dx + dy * dy
        };
        let part = self
            .parts
            .iter()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .ok_or(LayoutError::InvalidRange)?;
        Ok(part.range.start + part.page.hit_test(x - part.origin[0], y - part.origin[1])?)
    }

    pub fn selection_geometry(&self, range: Range<usize>) -> Result<Vec<Rect>, LayoutError> {
        let mut rects = Vec::new();
        for part in &self.parts {
            if let Some(local) = intersection(&range, &part.range) {
                rects.extend(
                    part.page
                        .selection_geometry(local)?
                        .into_iter()
                        .map(|rect| translate(rect, part.origin)),
                );
            }
        }
        Ok(rects)
    }

    pub fn line_at_y(&self, y: f32) -> Result<(Range<usize>, f32), LayoutError> {
        let distance = |part: &PlacedPage| {
            (part.origin[1] - y)
                .max(0.0)
                .max(y - part.origin[1] - part.page.height())
        };
        let part = self
            .parts
            .iter()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .ok_or(LayoutError::InvalidRange)?;
        let (range, top) = part.page.line_at_y(y - part.origin[1])?;
        let mut range = range.start + part.range.start..range.end + part.range.start;
        if let Some(row) = &part.table_row {
            // A table's visual line spans all its cells. Returning only the
            // leftmost cell truncates later columns at backward page boundaries.
            // Keep each cell's visual line, rather than its whole source range,
            // so a byte-cropped giant cell still makes progress in both directions.
            for cell in self
                .parts
                .iter()
                .filter(|cell| cell.table_row.as_ref() == Some(row) && !cell.range.is_empty())
            {
                let (line, _) = cell.page.line_at_y(y - cell.origin[1])?;
                range.start = range.start.min(cell.range.start + line.start);
                range.end = range.end.max(cell.range.start + line.end);
            }
        }
        Ok((range, top + part.origin[1]))
    }
}

fn translate(rect: Rect, origin: [f32; 2]) -> Rect {
    Rect {
        x0: rect.x0 + f64::from(origin[0]),
        x1: rect.x1 + f64::from(origin[0]),
        y0: rect.y0 + f64::from(origin[1]),
        y1: rect.y1 + f64::from(origin[1]),
    }
}
