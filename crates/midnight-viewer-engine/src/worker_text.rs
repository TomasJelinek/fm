//! Source coordinates and presentation prepared on the viewer worker.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Coordinate {
    pub source_offset: u64,
    pub line: u64,
    pub column: Option<u64>,
}

#[derive(Default)]
pub(super) struct Presentation {
    pub coordinate: Option<Coordinate>,
    pub locate_request: Option<(u64, u64)>,
    pub located_start: Option<u64>,
    pub unavailable: bool,
    pub gutter: Option<Vec<(TextPage, f32)>>,
}

impl Presentation {
    pub fn page(&mut self, previous: Option<&Page>, page: &Page) {
        // An optional bounded Locate may decline this page without disabling
        // cheap coordinates on a later nearby page.
        self.unavailable = false;
        self.located_start = None;
        self.coordinate = (!page.before)
            .then_some(Coordinate {
                source_offset: page.start,
                line: 1,
                column: Some(1),
            })
            .or_else(|| {
                let previous = previous?;
                let first = page
                    .anchors
                    .partition_point(|anchor| anchor.source_offset < previous.start);
                let transferred = page
                    .anchors
                    .get(first)
                    .filter(|anchor| anchor.source_offset <= previous.end)
                    .and_then(|anchor| {
                        previous
                            .anchors
                            .binary_search_by_key(&anchor.source_offset, |anchor| {
                                anchor.source_offset
                            })
                            .ok()
                    })
                    .and_then(|index| {
                        self.at(previous, previous.anchors[index].text_offset as usize)
                    });
                transferred.or_else(|| {
                    self.coordinate.filter(|coordinate| {
                        page.anchors
                            .binary_search_by_key(&coordinate.source_offset, |anchor| {
                                anchor.source_offset
                            })
                            .is_ok()
                    })
                })
            });
        self.gutter = None;
    }

    pub fn at(&self, page: &Page, index: usize) -> Option<Coordinate> {
        let mut position = self.coordinate?;
        let start = self.start_index(page)?;
        if index >= start {
            for character in page.text.get(start..index)?.chars() {
                if is_line_break(character) {
                    position.line = position.line.checked_add(1)?;
                    position.column = Some(1);
                } else {
                    position.column = position.column.and_then(|column| column.checked_add(1));
                }
            }
        } else {
            let between = page.text.get(index..start)?;
            let lines = between
                .chars()
                .filter(|character| is_line_break(*character))
                .count() as u64;
            position.line = position.line.checked_sub(lines)?;
            position.column = if lines == 0 {
                position
                    .column
                    .and_then(|column| column.checked_sub(between.chars().count() as u64))
            } else {
                // The first fragment's missing logical-line prefix is unknown;
                // keep Locate rather than inventing its global scalar column.
                let prefix = page.text.get(..index)?;
                prefix
                    .char_indices()
                    .rfind(|(_, c)| is_line_break(*c))
                    .map(|(at, newline)| {
                        prefix[at + newline.len_utf8()..].chars().count() as u64 + 1
                    })
            };
        }
        position.source_offset = page
            .anchors
            .iter()
            .find(|anchor| anchor.text_offset as usize == index)
            .map_or(position.source_offset, |anchor| anchor.source_offset);
        Some(position)
    }

    fn start_index(&self, page: &Page) -> Option<usize> {
        let position = self.coordinate?;
        if position.source_offset == page.start {
            Some(0)
        } else {
            page.anchors
                .iter()
                .find(|anchor| anchor.source_offset == position.source_offset)
                .map(|anchor| anchor.text_offset as usize)
        }
    }

    fn line_starts(&self, page: &Page) -> Vec<(usize, u64)> {
        let (Some(position), Some(start)) = (self.coordinate, self.start_index(page)) else {
            return Vec::new();
        };
        let Some(mut line) = position.line.checked_sub(
            page.text[..start]
                .chars()
                .filter(|character| is_line_break(*character))
                .count() as u64,
        ) else {
            return Vec::new();
        };
        let mut starts = vec![(0, line)];
        for (at, character) in page.text.char_indices() {
            if is_line_break(character) {
                let Some(next) = line.checked_add(1) else {
                    break;
                };
                line = next;
                starts.push((at + character.len_utf8(), line));
            }
        }
        starts
    }
}

fn is_line_break(character: char) -> bool {
    matches!(character, '\n' | '\u{2028}' | '\u{2029}')
}

impl Worker {
    pub(super) fn cancel_locate(&mut self) {
        if let Some((request, _)) = self.text.locate_request.take()
            && let Some(session) = &self.session
        {
            session.cancel(request);
        }
    }

    pub(super) fn locate_source(&mut self) {
        if self.mode != viewer::MODE_SOURCE
            || self.page.as_ref().is_some_and(|page| {
                self.text.coordinate.is_some_and(|coordinate| {
                    coordinate.source_offset == page.start && coordinate.column.is_some()
                }) || self.text.located_start == Some(page.start)
            })
            || self.text.unavailable
            || self.text.locate_request.is_some()
            || self.viewport_request.is_some()
        {
            return;
        }
        if let (Some(session), Some(page)) = (&self.session, &self.page)
            && let Ok(id) = session.locate(messages::Locate {
                source_anchor: page.start,
            })
        {
            self.text.locate_request = Some((id, page.start));
        }
    }

    pub(super) fn located(&mut self, id: u64, position: &viewer::TextPosition) {
        if let Some((request, start)) = self.text.locate_request
            && request == id
            && let Some(page) = &self.page
            && page.start == start
            && position.source_offset >= start
            && position.source_offset <= page.end
            && (position.source_offset == start
                || page
                    .anchors
                    .iter()
                    .any(|anchor| anchor.source_offset == position.source_offset))
        {
            self.text.coordinate = Some(Coordinate {
                source_offset: position.source_offset,
                line: position.line,
                column: Some(position.column),
            });
            self.text.gutter = None;
            self.raster.invalidate();
            self.dirty = true;
        }
    }

    pub(super) fn text_margin(&self) -> f32 {
        let width = self.geometry.width as f32;
        let scale = self.geometry.scale;
        if self.mode == viewer::MODE_SOURCE && self.preferences.line_numbers {
            // File length bounds its possible line count. Stable across viewport
            // arrivals and asynchronous indexing, including unnumbered fragments.
            let digits = self.length.max(999).ilog10() + 1;
            ((digits as f32 + 2.0) * self.appearance.font_size * 0.65 * scale)
                .ceil()
                .min((width - 1.0).max(0.0))
        } else if self.mode == viewer::MODE_DOCUMENT {
            let (padding, maximum) = match self.preferences.markdown_preset {
                MarkdownPreset::Compact => (8.0, f32::MAX),
                MarkdownPreset::GitHub => (if width / scale >= 768.0 { 32.0 } else { 16.0 }, 980.0),
                MarkdownPreset::Comfortable => (28.0, 760.0),
            };
            (padding * scale)
                .max((width - maximum * scale) / 2.0)
                .min(((width - 1.0) / 2.0).max(0.0))
        } else {
            0.0
        }
    }

    pub(super) fn text_width(&self) -> f32 {
        let margin = self.text_margin();
        (self.geometry.width as f32
            - margin
                * if self.mode == viewer::MODE_DOCUMENT {
                    2.0
                } else {
                    1.0
                })
        .max(1.0)
    }

    pub(super) fn line_height(&self) -> Option<f32> {
        (self.mode == viewer::MODE_DOCUMENT).then_some(match self.preferences.markdown_preset {
            MarkdownPreset::Compact => 1.25,
            MarkdownPreset::GitHub => 1.5,
            MarkdownPreset::Comfortable => 1.8,
        })
    }

    pub(super) fn source_status(&self, page: &Page, shaped: &TextPage) -> String {
        if self.mode != viewer::MODE_SOURCE {
            return String::new();
        }
        let index = self
            .selection
            .map(|(_, end)| end)
            .or_else(|| {
                shaped
                    .line_at_y(self.origin[1] + 0.5)
                    .ok()
                    .map(|(range, _)| range.start)
            })
            .unwrap_or(0);
        match self.text.at(page, index) {
            Some(Coordinate {
                line,
                column: Some(column),
                ..
            }) => format!("Ln {line}, Col {column}  "),
            Some(Coordinate {
                line, column: None, ..
            }) => format!("Ln {line}, Col unavailable  "),
            None if self.text.unavailable
                || (self.text.coordinate.is_some() && self.text.locate_request.is_none()) =>
            {
                "Line/column unavailable  ".into()
            }
            None => "Line/column indexing…  ".into(),
        }
    }

    pub(super) fn prepare_gutter(&mut self) -> Result<(), String> {
        if self.mode != viewer::MODE_SOURCE
            || !self.preferences.line_numbers
            || self.text.gutter.is_some()
        {
            return Ok(());
        }
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return Ok(());
        };
        let options = TextOptions {
            family: "monospace",
            size: self.appearance.font_size,
            scale: self.geometry.scale,
            foreground: self.appearance.foreground,
            ..TextOptions::default()
        };
        let mut labels = Vec::new();
        for (index, line) in self.text.line_starts(page) {
            let y = shaped
                .caret_geometry(index)
                .map_err(|error| error.to_string())?
                .y0 as f32;
            let label = self
                .renderer
                .layout_with_options(&line.to_string(), &options, &[])
                .map_err(|error| error.to_string())?;
            labels.push((label, y));
        }
        self.text.gutter = Some(labels);
        Ok(())
    }
}

pub(super) fn paint_gutter(
    renderer: &mut TextRenderer,
    text: &Presentation,
    appearance: &ViewerAppearance,
    geometry: ViewerGeometry,
    origin: [f32; 2],
    margin: f32,
    raster: &mut midnight_viewer_render::Raster,
) -> Result<(), String> {
    let width = (margin as usize).min(raster.width as usize);
    // The app theme uses opaque backgrounds; still preserve premultiplication.
    let background = appearance.background;
    let color = [
        ((u16::from(background[0]) * u16::from(background[3]) + 127) / 255) as u8,
        ((u16::from(background[1]) * u16::from(background[3]) + 127) / 255) as u8,
        ((u16::from(background[2]) * u16::from(background[3]) + 127) / 255) as u8,
        background[3],
    ];
    for row in raster.rgba.chunks_exact_mut(raster.width as usize * 4) {
        for pixel in row[..width * 4].as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&color);
        }
    }
    if let Some(labels) = &text.gutter {
        for (label, y) in labels {
            if *y + label.height() < origin[1] || *y > origin[1] + raster.height as f32 {
                continue;
            }
            let x = (margin - label.width() - 8.0 * geometry.scale).max(0.0);
            renderer
                .paint_over(label, raster, [-x, origin[1] - *y])
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
