use crate::scroll_view::{self, HORIZONTAL_LIMIT, MAX_ROWS, Surface};
use crate::{
    Frame, Palette, TextNavigation, check_cancel, identity::DecodedText, typeface::Typeface,
};
use midnight_core::{ViewerAppearance, ViewerGeometry};
use midnight_plugin_host::CapturedFile;
use midnight_viewer_render::{TextOptions, TextPage, TextRenderer, Viewport};
use similar::{DiffOp, DiffTag};
use std::sync::{Arc, atomic::AtomicBool};

#[path = "text_align.rs"]
mod align;
#[path = "text_find.rs"]
mod find;
#[path = "text_inline.rs"]
mod inline;

/// Larger files, and pairs that are not text, are compared as bytes. Measurements are in
/// docs/developer/FILE_COMPARISON.md.
pub(crate) const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The alignment's cost follows the lines, not the bytes: this bound, unchanged when the
/// byte limit doubled from 32 MiB, keeps the worst case of the earlier measurements. A
/// 64 MiB file holds at most this many lines when they average 67 bytes or more; one
/// with more lines is compared as bytes.
pub(crate) const MAX_LINES: usize = 1_000_000;
/// Each visible row is shaped whole, so this bounds the work of one paint.
const MAX_LINE_BYTES: usize = 16 * 1024;
const READ_BLOCK: u32 = 256 * 1024;
/// Reading reports its progress after each such amount.
const PROGRESS_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16LittleBom,
    Utf16BigBom,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8 without BOM",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::Utf16LittleBom => "UTF-16 little-endian with BOM",
            Self::Utf16BigBom => "UTF-16 big-endian with BOM",
        }
    }
}

/// Why a file was not read as text.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// The content or its size is outside what text comparison admits; the pair is then
    /// compared as bytes, with this reason shown.
    Refused(String),
    /// Reading failed: cancellation, an I/O error or a changed file. The comparison fails.
    Failed(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(reason) | Self::Failed(reason) => output.write_str(reason),
        }
    }
}

/// One side's decoded text and the end offset of each of its lines, line breaks
/// included. Offsets fit in `u32` because admitted text is far below 4 GiB.
pub(crate) struct TextLines {
    text: String,
    ends: Vec<u32>,
}

impl TextLines {
    /// Split admitted text, refusing more than `MAX_LINES` lines or a line longer than
    /// `MAX_LINE_BYTES`.
    /// The line ends of a refused text are never all kept: at most `MAX_LINES` of them.
    fn split(text: String) -> Result<Self, String> {
        let mut ends = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            if ends.len() == MAX_LINES {
                return Err(format!(
                    "text comparison supports up to {} lines per file",
                    grouped(MAX_LINES as u64)
                ));
            }
            if line.len() > MAX_LINE_BYTES {
                return Err("text comparison supports lines up to 16 KiB".into());
            }
            offset += line.len();
            ends.push(offset as u32);
        }
        Ok(Self { text, ends })
    }

    /// The lines of any text, without the limits (tests).
    #[cfg(test)]
    fn unchecked(text: String) -> Self {
        let mut ends = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            offset += line.len();
            ends.push(offset as u32);
        }
        Self { text, ends }
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        let end = *self.ends.get(index)? as usize;
        let start = index
            .checked_sub(1)
            .map_or(0, |previous| self.ends[previous] as usize);
        self.text.get(start..end)
    }

    fn line(&self, index: usize) -> &str {
        self.get(index).unwrap_or_default()
    }
}

pub(crate) struct TextInput {
    pub lines: TextLines,
    pub encoding: TextEncoding,
}

/// One aligned row: a line index on each side (`ABSENT` for none) and whether it changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    left: u32,
    right: u32,
    changed: bool,
}

const ABSENT: u32 = u32::MAX;

struct DifferencePosition {
    previous_row: Option<usize>,
    next_row: Option<usize>,
    /// One-based group number when the top row belongs to a difference.
    current_group: Option<usize>,
}

impl Row {
    fn side(&self, side: usize) -> Option<usize> {
        let index = if side == 0 { self.left } else { self.right };
        (index != ABSENT).then_some(index as usize)
    }

    fn left(&self) -> Option<usize> {
        self.side(0)
    }

    fn right(&self) -> Option<usize> {
        self.side(1)
    }
}

pub(crate) struct TextComparison {
    find: find::TextFind,
    lines: [TextLines; 2],
    rows: Vec<Row>,
    differences: Vec<usize>,
    /// On opening or jumping, bring the first edited column into view once its font is known.
    reveal_difference: bool,
    renderer: TextRenderer,
    pub typeface: Typeface,
    pub top: usize,
    /// Physical pixels scrolled to the right.
    pub horizontal: f32,
    /// The part of the `top` row above the view (0 to 1).
    fraction: f64,
    /// Logical row height, fully visible rows and scale of the last painted frame.
    row_height: f32,
    page_rows: usize,
    scale: f32,
    painted_width: u32,
    decoded_identity: DecodedText,
    pub encodings: Option<[TextEncoding; 2]>,
    /// The line alignment ran out of time: some unchanged lines may show as changed.
    approximate: bool,
}

/// Each 256 KiB block is admitted by the storage policy: shared on a confirmed local SSD,
/// exclusive otherwise, so a long copy cannot hold back comparisons of SSD files.
/// `progress` receives the bytes read so far after every few MiB of a large file.
pub(crate) fn read(
    file: &CapturedFile,
    confirmed_ssd: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<TextInput, ReadError> {
    let length = file.length();
    if length > MAX_BYTES {
        return Err(ReadError::Refused(format!(
            "text comparison supports files up to {} MiB",
            MAX_BYTES >> 20
        )));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    let mut reported = 0;
    while (bytes.len() as u64) < length {
        check_cancel(cancel).map_err(ReadError::Failed)?;
        let budget = midnight_work_budget::shared_io();
        let _permit = budget
            .acquire(
                midnight_work_budget::storage_admission(confirmed_ssd),
                || cancel.load(std::sync::atomic::Ordering::Acquire),
            )
            .ok_or_else(|| ReadError::Failed("Comparison cancelled.".into()))?;
        let amount = (length - bytes.len() as u64).min(u64::from(READ_BLOCK)) as u32;
        let block = file
            .read_at(bytes.len() as u64, amount)
            .map_err(|error| ReadError::Failed(error.to_string()))?;
        if block.len() != amount as usize {
            return Err(ReadError::Failed(
                "Compared file returned a short read.".into(),
            ));
        }
        // Most binary files show a control byte early: do not read them whole as text.
        if bytes.is_empty() && binary_start(&block) {
            return Err(ReadError::Refused(NOT_TEXT.into()));
        }
        bytes.extend(block);
        if bytes.len() as u64 - reported >= PROGRESS_BYTES {
            reported = bytes.len() as u64;
            progress(reported);
        }
    }
    file.revalidate()
        .map_err(|error| ReadError::Failed(error.to_string()))?;
    let encoding = if bytes.starts_with(&[0xff, 0xfe]) {
        TextEncoding::Utf16LittleBom
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        TextEncoding::Utf16BigBom
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        TextEncoding::Utf8Bom
    } else {
        TextEncoding::Utf8
    };
    let text = decode_owned(bytes).map_err(ReadError::Refused)?;
    check_cancel(cancel).map_err(ReadError::Failed)?;
    Ok(TextInput {
        lines: TextLines::split(text).map_err(ReadError::Refused)?,
        encoding,
    })
}

const NOT_TEXT: &str = "not UTF-8 or UTF-16 text (binary or another encoding)";

/// A first block without a UTF-16 byte order mark that holds a byte no admitted text has.
fn binary_start(block: &[u8]) -> bool {
    !(block.starts_with(&[0xff, 0xfe]) || block.starts_with(&[0xfe, 0xff]))
        && block
            .iter()
            .any(|&byte| byte == 0x7f || (byte < 0x20 && !text_control(char::from(byte))))
}

/// Decode admitted text and check its lines, as `read` does after reading.
#[cfg(test)]
fn decode(bytes: &[u8]) -> Result<String, String> {
    let text = decode_owned(bytes.to_vec())?;
    TextLines::split(text).map(|lines| lines.text)
}

fn decode_owned(bytes: Vec<u8>) -> Result<String, String> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err("invalid UTF-16 text: incomplete code unit".into());
        }
        let little = bytes[0] == 0xff;
        let words: Vec<_> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|word| {
                if little {
                    u16::from_le_bytes([word[0], word[1]])
                } else {
                    u16::from_be_bytes([word[0], word[1]])
                }
            })
            .collect();
        String::from_utf16(&words).map_err(|_| "invalid UTF-16 text")?
    } else {
        let mut bytes = bytes;
        if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            bytes.drain(..3);
        }
        String::from_utf8(bytes).map_err(|_| NOT_TEXT)?
    };
    if has_binary_control(&text) {
        return Err("contains binary control characters".into());
    }
    Ok(text)
}

/// Whether `text` has a control character (`char::is_control`) that `text_control` does
/// not admit, checked on its UTF-8 bytes eight at a time: C0 controls and DEL are single
/// bytes, and the C1 controls U+0080–U+009F are 0xC2 0x80–0x9F.
fn has_binary_control(text: &str) -> bool {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    let zero = |word: u64| word.wrapping_sub(ONES) & !word & HIGH != 0;
    let bytes = text.as_bytes();
    let binary = |index: usize| match bytes[index] {
        byte if byte < 0x20 => !text_control(char::from(byte)),
        0x7f => true,
        0xc2 => bytes
            .get(index + 1)
            .is_some_and(|next| (0x80..=0x9f).contains(next)),
        _ => false,
    };
    let (words, _) = bytes.as_chunks::<8>();
    for (index, word) in words.iter().enumerate() {
        let word = u64::from_le_bytes(*word);
        // Any byte below 0x20, equal to 0x7F or equal to 0xC2 (never a false negative).
        let candidate = word.wrapping_sub(ONES * 0x20) & !word & HIGH != 0
            || zero(word ^ (ONES * 0x7f))
            || zero(word ^ (ONES * 0xc2));
        if candidate && (index * 8..index * 8 + 8).any(binary) {
            return true;
        }
    }
    (words.len() * 8..bytes.len()).any(binary)
}

impl TextComparison {
    /// Compare two texts without the reading limits (tests and embedding).
    #[cfg(test)]
    pub fn new(
        left: impl Into<String>,
        right: impl Into<String>,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        Self::from_lines(
            [
                TextLines::unchecked(left.into()),
                TextLines::unchecked(right.into()),
            ],
            cancel,
        )
    }

    pub fn from_lines(lines: [TextLines; 2], cancel: &AtomicBool) -> Result<Self, String> {
        check_cancel(cancel)?;
        let alignment = align::align(&lines, cancel)?;
        let mut rows = Vec::new();
        let mut differences = Vec::new();
        for op in &alignment.ops {
            check_cancel(cancel)?;
            push_rows(op, &mut rows, &mut differences);
        }
        let top = differences.first().copied().unwrap_or(0);
        Ok(Self {
            find: find::TextFind::default(),
            lines,
            rows,
            reveal_difference: !differences.is_empty(),
            differences,
            renderer: TextRenderer::default(),
            typeface: Typeface::default(),
            top,
            horizontal: 0.0,
            fraction: 0.0,
            row_height: 24.0,
            page_rows: 1,
            scale: 1.0,
            painted_width: 0,
            decoded_identity: DecodedText::default(),
            encodings: None,
            approximate: alignment.approximate,
        })
    }

    pub fn identity_turn(&mut self) -> Option<bool> {
        self.decoded_identity
            .step([self.lines[0].text(), self.lines[1].text()])
    }

    /// Whether `input` is exactly the text and encoding retained for `side`. Decoding is
    /// strict, so equal text and encoding mean equal file bytes.
    pub fn retains(&self, side: usize, input: &TextInput) -> bool {
        self.encodings
            .is_some_and(|encodings| encodings[side] == input.encoding)
            && self.lines[side].text() == input.lines.text()
    }

    pub fn find_pending(&self) -> bool {
        self.find.pending()
    }

    /// Preserve a visible Find result across a resize. The current scroll positions,
    /// rather than only the last frame's state, keep intentional manual scrolling intact.
    pub fn reveal_visible_find_on_resize(&mut self) {
        let Some(hit) = &self.find.selected else {
            return;
        };
        let Some([left, right]) = self.find.horizontal_bounds else {
            return;
        };
        let pane_width = if hit.side == 0 {
            self.painted_width / 2
        } else {
            self.painted_width - self.painted_width / 2
        };
        if hit.row >= self.top
            && hit.row < self.top.saturating_add(self.page_rows)
            && right > f64::from(self.horizontal)
            && left < f64::from(self.horizontal) + f64::from(pane_width)
        {
            self.find.reveal = true;
        }
    }

    fn last_row(&self) -> usize {
        self.rows.len().saturating_sub(1)
    }

    pub fn scroll(&mut self, rows: i64) {
        self.reveal_difference = false;
        self.top = self
            .top
            .saturating_add_signed(rows as isize)
            .min(self.last_row());
        if self.top == self.last_row() {
            self.fraction = 0.0;
        }
    }
    /// The view moves by exactly these logical pixels (negative: down), within its rows.
    pub fn scroll_pixels(&mut self, pixels: f32) {
        self.reveal_difference = false;
        let (top, fraction) = scroll_view::scroll_rows(
            self.top as u64,
            self.fraction,
            pixels,
            self.row_height,
            self.last_row() as u64,
        );
        self.top = top as usize;
        self.fraction = fraction;
    }
    /// Logical pixels to the right (negative: right), as a wheel or a key moves them.
    pub fn scroll_horizontal(&mut self, pixels: f32) {
        if pixels.is_finite() {
            self.reveal_difference = false;
            self.horizontal = (self.horizontal - pixels * self.scale).clamp(0.0, HORIZONTAL_LIMIT);
        }
    }
    /// A page moves by the rows that were fully visible, never skipping or repeating one.
    pub fn scroll_pages(&mut self, pages: i32) {
        self.fraction = 0.0;
        self.scroll(i64::from(pages).saturating_mul(self.page_rows as i64));
    }
    /// The position bar: that share of the rows, to the pixel.
    pub fn seek(&mut self, amount: f64) {
        if amount.is_finite() {
            self.reveal_difference = false;
            let (top, fraction) = scroll_view::seek_rows(amount, self.last_row() as u64);
            self.top = top as usize;
            self.fraction = fraction;
        }
    }
    fn difference_position(&self) -> DifferencePosition {
        let after = self.differences.partition_point(|&start| start <= self.top);
        let (through_top, below) = self.differences.split_at(after);
        // A partly scrolled-off group start is reachable with Previous. When it is
        // exactly at the top, Previous goes to the preceding group.
        let previous_row = match through_top.split_last() {
            Some((&start, earlier)) if start == self.top && self.fraction == 0.0 => {
                earlier.last().copied()
            }
            _ => through_top.last().copied(),
        };
        DifferencePosition {
            previous_row,
            next_row: below.first().copied(),
            current_group: self
                .rows
                .get(self.top)
                .filter(|row| row.changed)
                .map(|_| after),
        }
    }

    pub fn difference(&mut self, backwards: bool) {
        let position = self.difference_position();
        let target = if backwards {
            position.previous_row
        } else {
            position.next_row
        };
        if let Some(target) = target {
            self.top = target;
            self.fraction = 0.0;
            self.reveal_difference = true;
        }
    }

    pub fn difference_edge(&mut self, last: bool) {
        if let Some(target) = if last {
            self.differences.last()
        } else {
            self.differences.first()
        } {
            self.top = *target;
            self.fraction = 0.0;
            self.reveal_difference = true;
        }
    }

    pub fn paint(
        &mut self,
        geometry: ViewerGeometry,
        appearance: &ViewerAppearance,
        palette: Palette,
        cancel: &AtomicBool,
    ) -> Result<Frame, String> {
        let family = self.typeface.painted().to_owned();
        let family = family.as_str();
        let options = TextOptions {
            family,
            literal_family: family != "monospace",
            size: appearance.font_size,
            scale: geometry.scale,
            foreground: palette.foreground,
            ..TextOptions::default()
        };
        let (width, height) = (geometry.width, geometry.height);
        let row_height = (appearance.font_size * geometry.scale * 1.6)
            .ceil()
            .max(12.0) as u32;
        self.row_height = row_height as f32 / geometry.scale;
        self.scale = geometry.scale;
        self.page_rows = (height / row_height).max(1) as usize;
        if self.find.reveal {
            if let Some(hit) = &self.find.selected {
                self.top = hit.row.saturating_sub(self.page_rows / 2);
                self.fraction = 0.0;
                let row = &self.rows[hit.row];
                let (value, prefix) = self.formatted_line(hit.side, hit.line, row.changed);
                let page = self
                    .renderer
                    .layout_with_options(&value, &options, &[])
                    .map_err(|error| format!("Cannot lay out Find result: {error:?}"))?;
                let bounds =
                    shaped_hit_bounds(&page, prefix + hit.range.start..prefix + hit.range.end)?;
                self.find.horizontal_bounds = Some(bounds);
                let pane_width = if hit.side == 0 {
                    width / 2
                } else {
                    width - width / 2
                };
                self.horizontal =
                    ((bounds[0] + bounds[1] - f64::from(pane_width)) / 2.0).max(0.0) as f32;
            }
            self.find.reveal = false;
        } else if self.reveal_difference {
            self.reveal_changed_column(width, &options)?;
        }
        self.reveal_difference = false;
        self.painted_width = width;
        self.top = self.top.min(self.last_row());
        if self.top == self.last_row() {
            self.fraction = 0.0;
        }
        let row = u64::from(row_height);
        let surface = Surface::plan(
            width,
            height,
            geometry.scale,
            row_height,
            [
                self.horizontal.round().max(0.0) as u64,
                self.top as u64 * row + scroll_view::fraction_pixels(self.fraction, row_height),
            ],
            [HORIZONTAL_LIMIT as u64, self.last_row() as u64 * row],
        );
        let surface_width = surface.width() as usize;
        let surface_height = u64::from(surface.surface_height());
        let mut pixels = palette
            .background
            .repeat(surface_width * surface_height as usize);
        // The view's rows, for assistive technology; the margins are only painted.
        let view = surface.margin[1][0] as u64..surface.margin[1][0] as u64 + u64::from(height);
        let origin = self.horizontal - surface.margin[0][0] as f32;
        let first = (surface.top() / row) as usize;
        let mut accessible_text = String::new();
        for index in first..self.rows.len().min(first + MAX_ROWS as usize) {
            // The row's first pixel row in the surface; the first one may start above it.
            let y = (index as u64 * row) as i64 - surface.top() as i64;
            if y >= surface_height as i64 {
                break;
            }
            check_cancel(cancel)?;
            let row_value = self.rows[index];
            let shown = (y + row as i64) as u64 > view.start && (y.max(0) as u64) < view.end;
            let formatted = self.formatted_row(row_value);
            let changed_ranges =
                (row_value.changed && row_value.left().is_some() && row_value.right().is_some())
                    .then(|| {
                        let [(left, a), (right, b)] = &formatted;
                        inline::changed_ranges(&left[*a..], &right[*b..])
                    });
            for (side, (value, prefix)) in formatted.into_iter().enumerate() {
                let (left, side_width) = surface.region(side);
                let selection = self
                    .find
                    .selected
                    .as_ref()
                    .filter(|hit| hit.row == index && hit.side == side)
                    .map(|hit| prefix + hit.range.start..prefix + hit.range.end);
                if shown {
                    accessible_text.push_str(&value);
                    accessible_text.push(if side == 0 { '\t' } else { '\n' });
                }
                let background = if row_value.changed {
                    changed_background(palette.background, side)
                } else {
                    palette.background
                };
                let page = self
                    .renderer
                    .layout_with_options(&value, &options, &[])
                    .map_err(|error| format!("Cannot lay out comparison: {error:?}"))?;
                let (highlight, fill, ink) = if let Some(range) = selection {
                    self.find.horizontal_bounds = Some(shaped_hit_bounds(&page, range.clone())?);
                    (Some(range), palette.selection, palette.selection_text)
                } else {
                    let span = changed_ranges
                        .as_ref()
                        .map(|ranges| &ranges[side])
                        .filter(|span| !span.is_empty())
                        .map(|span| prefix + span.start..prefix + span.end);
                    // Match the hex comparator's stronger tint. The renderer keeps the
                    // glyph color readable on it, including for high contrast themes.
                    (
                        span,
                        tinted(palette.background, side, 3, 5),
                        palette.foreground,
                    )
                };
                let raster = self
                    .renderer
                    .rasterize(
                        &page,
                        Viewport {
                            width: side_width,
                            height: row_height,
                            origin: [origin, 0.0],
                            background,
                            selection_color: fill,
                            selection_text: ink,
                        },
                        highlight,
                    )
                    .map_err(|error| format!("Cannot render comparison: {error:?}"))?;
                copy_row(
                    &mut pixels,
                    surface_width,
                    surface_height,
                    &raster.rgba,
                    (left, side_width),
                    y,
                );
            }
        }
        let position = self.difference_position();
        let total = self.differences.len();
        let mut status = match (position.current_group, total) {
            (Some(group), _) => format!("Difference {group} of {total}"),
            (_, 0) => "No line differences".into(),
            (_, 1) => "1 difference group".into(),
            _ => format!("{total} difference groups"),
        };
        if !self.rows.is_empty() {
            use std::fmt::Write;
            let _ = write!(status, " · row {} of {}", self.top + 1, self.rows.len());
        }
        status.push_str(" · live-validated read");
        if self.approximate {
            status.push_str(" · line matching timed out, so some unchanged lines show as changed");
        }
        Ok(Frame {
            find: Arc::clone(&self.find.state),
            fonts: self.typeface.fonts.clone(),
            #[cfg(test)]
            text_family: Some(family.to_owned()),
            geometry_revision: 0,
            comparison_generation: 0,
            identity: crate::IdentityVerdict::Pending,
            mode: None,
            width: surface.width(),
            height: surface.surface_height(),
            pixels,
            layers: None,
            accessible_text,
            status,
            image: false,
            bytes: false,
            bytes_only: false,
            scanning: false,
            position: surface.position[1] as f64 / surface.extent[1].max(1) as f64,
            text_navigation: Some(TextNavigation {
                previous: position.previous_row.is_some(),
                next: position.next_row.is_some(),
            }),
            scroll: Some(Box::new(surface.view(geometry.scale))),
        })
    }

    fn reveal_changed_column(
        &mut self,
        width: u32,
        options: &TextOptions<'_>,
    ) -> Result<(), String> {
        let row = self.rows[self.top];
        if row.left().is_none() || row.right().is_none() {
            // The entire line was inserted or removed.
            self.horizontal = 0.0;
            return Ok(());
        }
        let formatted = self.formatted_row(row);
        let [(left, a), (right, b)] = &formatted;
        let ranges = inline::changed_ranges(&left[*a..], &right[*b..]);
        let side = usize::from(ranges[0].is_empty());
        let (value, prefix) = &formatted[side];
        let start = ranges[side].start;
        if ranges[side].is_empty() {
            return Ok(());
        }
        let page = self
            .renderer
            .layout_with_options(value, options, &[])
            .map_err(|error| format!("Cannot lay out changed line: {error:?}"))?;
        let caret = page
            .caret_geometry(prefix + start)
            .map_err(|error| format!("Cannot position changed text: {error:?}"))?;
        let pane_width = if side == 0 {
            width / 2
        } else {
            width - width / 2
        } as f32;
        let column = caret.x0 as f32;
        // Leave room to read past the beginning of the edit if it is at the right edge.
        if column < self.horizontal
            || column + options.size * options.scale > self.horizontal + pane_width
        {
            self.horizontal = (column - pane_width / 3.0).clamp(0.0, HORIZONTAL_LIMIT);
        }
        Ok(())
    }

    fn formatted_row(&self, row: Row) -> [(String, usize); 2] {
        [0, 1].map(|side| {
            row.side(side)
                .map(|line| self.formatted_line(side, line, row.changed))
                .unwrap_or_default()
        })
    }

    fn formatted_line(&self, side: usize, index: usize, changed: bool) -> (String, usize) {
        let text = self.lines[side].line(index);
        let ending = if text.ends_with("\r\n") {
            "  [CRLF]"
        } else if text.ends_with('\n') {
            ""
        } else {
            "  [no newline]"
        };
        let marker = if changed {
            if side == 0 { "−" } else { "+" }
        } else {
            " "
        };
        let prefix = format!("{:>6} {marker} ", index + 1);
        let length = prefix.len();
        (
            format!(
                "{prefix}{}{ending}",
                visible_controls(text.trim_end_matches(['\r', '\n']))
            ),
            length,
        )
    }
}

fn push_rows(op: &DiffOp, rows: &mut Vec<Row>, differences: &mut Vec<usize>) {
    let changed = op.tag() != DiffTag::Equal;
    if changed {
        differences.push(rows.len());
    }
    let old = op.old_range();
    let new = op.new_range();
    for index in 0..old.len().max(new.len()) {
        rows.push(Row {
            left: if index < old.len() {
                (old.start + index) as u32
            } else {
                ABSENT
            },
            right: if index < new.len() {
                (new.start + index) as u32
            } else {
                ABSENT
            },
            changed,
        });
    }
}

/// Copy one painted row of `region` (its left edge and width) into a surface of
/// `width` × `height` pixels at row `y`, which may start above the surface.
pub(crate) fn copy_row(
    pixels: &mut [u8],
    width: usize,
    height: u64,
    row: &[u8],
    (left, region): (u32, u32),
    y: i64,
) {
    let stride = region as usize * 4;
    for (index, source) in row.chunks_exact(stride).enumerate() {
        let target_y = y + index as i64;
        if target_y < 0 {
            continue;
        }
        if target_y as u64 >= height {
            break;
        }
        let target = (target_y as usize * width + left as usize) * 4;
        pixels[target..target + stride].copy_from_slice(source);
    }
}

/// A changed row's background: the palette background tinted red (File A) or green
/// (File B).
pub(crate) fn changed_background(background: [u8; 4], side: usize) -> [u8; 4] {
    tinted(background, side, 1, 4)
}

/// `background` blended with the side's tint at `weight` parts of `total`.
pub(crate) fn tinted(background: [u8; 4], side: usize, weight: u16, total: u16) -> [u8; 4] {
    let tint = if side == 0 {
        [180u8, 45, 45, 255]
    } else {
        [40u8, 150, 75, 255]
    };
    std::array::from_fn(|channel| {
        if channel == 3 {
            255
        } else {
            ((u16::from(background[channel]) * (total - weight)
                + u16::from(tint[channel]) * weight)
                / total) as u8
        }
    })
}

/// `value` with thousands separators, for counts shown to people.
pub(crate) fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

fn shaped_hit_bounds(page: &TextPage, range: std::ops::Range<usize>) -> Result<[f64; 2], String> {
    let rectangles = page
        .selection_geometry(range.clone())
        .map_err(|error| format!("Cannot position Find result: {error:?}"))?;
    let bounds = rectangles
        .iter()
        .fold([f64::INFINITY, f64::NEG_INFINITY], |bounds, rect| {
            [bounds[0].min(rect.x0), bounds[1].max(rect.x1)]
        });
    if bounds[0].is_finite() && bounds[1].is_finite() {
        return Ok(bounds);
    }
    let start = page
        .caret_geometry(range.start)
        .map_err(|error| format!("Cannot position Find result: {error:?}"))?;
    let end = page
        .caret_geometry(range.end)
        .map_err(|error| format!("Cannot position Find result: {error:?}"))?;
    Ok([start.x0.min(end.x0), start.x0.max(end.x0)])
}

/// Text controls common in sources and logs: tab, line breaks, vertical tab, form feed
/// (page breaks) and escape (terminal colour codes). Other controls mark binary content.
fn text_control(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1b}')
}

/// Show the accepted invisible controls as Unicode control pictures (tabs keep their
/// layout), so a difference in them is visible and not rendered as tofu. The explicit
/// bidirectional embedding, override and isolate controls are shown as `\uNNNN` escapes,
/// so neither file can reorder its displayed lines ("Trojan Source"). Display only: the
/// diff compares the raw lines.
fn visible_controls(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains(|c| {
        matches!(c, '\u{0b}' | '\u{0c}' | '\u{1b}') || midnight_core::is_explicit_bidi_control(c)
    }) {
        return text.into();
    }
    let mut shown = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\u{0b}' | '\u{0c}' | '\u{1b}' => shown
                .push(char::from_u32(0x2400 + u32::from(c)).unwrap_or(char::REPLACEMENT_CHARACTER)),
            c if midnight_core::is_explicit_bidi_control(c) => {
                use std::fmt::Write;
                let _ = write!(shown, "\\u{:04X}", u32::from(c));
            }
            other => shown.push(other),
        }
    }
    shown.into()
}

#[cfg(test)]
#[path = "text_tests.rs"]
pub(crate) mod tests;
