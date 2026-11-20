//! Side-by-side byte comparison (hex view). A streaming scan of both captured files counts
//! every differing byte and keeps a bounded index of differing regions for navigation;
//! painting reads only the rows on screen. Owned and driven only by the comparison worker.
//!
//! A byte at offset `n` differs when the files hold different bytes there or only one file
//! is that long. A region is a maximal run of differing bytes.
use crate::{
    Frame, IdentityVerdict, Palette, check_cancel,
    identity::IdentityTurn,
    text::{changed_background, grouped, tinted},
    typeface::Typeface,
};
use midnight_core::{ViewerAppearance, ViewerGeometry};
use midnight_plugin_host::CapturedFile;
use midnight_viewer_render::{RasterMut, TextOptions, TextRenderer};
use midnight_work_budget::{Admission, Gate, storage_admission};
use std::{
    fmt::Write,
    ops::Range,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

/// Bytes compared per source in one scan turn: the native filesystem's largest read.
const SCAN_BYTES: u32 = 256 * 1024;
/// Region starts kept for navigation (8 bytes each). Beyond it the index keeps only
/// starts at least a growing spacing apart, so nearby regions share a stop.
const MAX_STOPS: usize = 1 << 18;
/// The scan's progress is painted at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// Bytes per row, widest first; a narrow pane uses the widest that fits.
const ROW_BYTES: [u64; 3] = [16, 8, 4];
/// At most this many rows are painted, like text comparison.
const MAX_ROWS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Jump {
    First,
    /// The first region on a row after this one.
    After(u64),
    Last,
}

/// Region starts for navigation, sorted, at most `MAX_STOPS` of them.
struct Stops {
    starts: Vec<u64>,
    /// Minimum distance between kept starts; 1 keeps every region.
    spacing: u64,
}

impl Default for Stops {
    fn default() -> Self {
        Self {
            starts: Vec::new(),
            spacing: 1,
        }
    }
}

impl Stops {
    fn push(&mut self, start: u64) {
        if self
            .starts
            .last()
            .is_some_and(|&last| start - last < self.spacing)
        {
            return;
        }
        self.starts.push(start);
        if self.starts.len() > MAX_STOPS {
            // Halve at once, so thinning stays rare however many regions follow.
            while self.starts.len() > MAX_STOPS / 2 {
                self.spacing = self.spacing.saturating_mul(2);
                let spacing = self.spacing;
                let mut kept: Option<u64> = None;
                self.starts.retain(|&start| {
                    let keep = kept.is_none_or(|kept| start - kept >= spacing);
                    if keep {
                        kept = Some(start);
                    }
                    keep
                });
            }
        }
    }
}

/// The streaming comparison of both files: exact counts and the navigation index.
pub(crate) struct Scan {
    lengths: [u64; 2],
    revisions: [u64; 2],
    /// Every byte before this offset is accounted for.
    offset: u64,
    /// The byte before `offset` differs, so a region continues.
    in_region: bool,
    differing: u64,
    regions: u64,
    first: Option<u64>,
    stops: Stops,
    complete: bool,
    gate: Arc<Gate>,
    admission: Admission,
    reported: Instant,
}

impl Scan {
    pub fn new(sources: &[CapturedFile; 2], confirmed_ssd: [bool; 2]) -> Self {
        Self {
            lengths: sources.each_ref().map(CapturedFile::length),
            revisions: sources.each_ref().map(CapturedFile::revision),
            offset: 0,
            in_region: false,
            differing: 0,
            regions: 0,
            first: None,
            stops: Stops::default(),
            complete: false,
            gate: midnight_work_budget::shared_io(),
            admission: storage_admission(confirmed_ssd[0] && confirmed_ssd[1]),
            reported: Instant::now(),
        }
    }

    pub fn complete(&self) -> bool {
        self.complete
    }

    pub fn verdict(&self) -> IdentityVerdict {
        if self.differing == 0 {
            IdentityVerdict::IdenticalFiles
        } else {
            IdentityVerdict::Different
        }
    }

    /// One non-queuing I/O admission and at most one 256 KiB read of each file, like
    /// whole-input verification (which reads 64 KiB). The tail of a longer file differs
    /// without being read. Both captured revisions are validated before completion.
    pub fn step(
        &mut self,
        sources: &[CapturedFile; 2],
        cancel: &AtomicBool,
        is_current: impl Fn() -> bool,
    ) -> Result<IdentityTurn, String> {
        if self.complete {
            return Ok(IdentityTurn::Complete(self.verdict()));
        }
        check_current(cancel, &is_current)?;
        if self.revisions != sources.each_ref().map(CapturedFile::revision) {
            return Err("The byte comparison belongs to a different captured pair.".into());
        }
        let Some(_permit) = self.gate.try_acquire(self.admission) else {
            return Ok(IdentityTurn::Blocked);
        };
        let common = self.lengths[0].min(self.lengths[1]);
        if self.offset < common {
            let amount = (common - self.offset).min(u64::from(SCAN_BYTES)) as u32;
            let mut blocks = Vec::with_capacity(2);
            for source in sources {
                let block = source.read_at(self.offset, amount).map_err(|error| {
                    format!("Compared file changed or could not be read: {error}")
                })?;
                check_current(cancel, &is_current)?;
                if block.len() != amount as usize {
                    return Err("Compared file returned a short read.".into());
                }
                blocks.push(block);
            }
            self.compare(self.offset, &blocks[0], &blocks[1]);
            self.offset += u64::from(amount);
            if self.offset < common {
                return Ok(IdentityTurn::Advanced);
            }
        }
        let total = self.lengths[0].max(self.lengths[1]);
        if self.offset < total {
            self.mark_tail(self.offset, total - self.offset);
            self.offset = total;
        }
        for source in sources {
            check_current(cancel, &is_current)?;
            source
                .revalidate()
                .map_err(|error| format!("Compared file changed: {error}"))?;
        }
        check_current(cancel, &is_current)?;
        self.complete = true;
        Ok(IdentityTurn::Complete(self.verdict()))
    }

    /// Whether the progress shown is old enough to paint again.
    pub fn progress_due(&mut self) -> bool {
        if self.reported.elapsed() < PROGRESS_INTERVAL {
            return false;
        }
        self.reported = Instant::now();
        true
    }

    fn compare(&mut self, offset: u64, left: &[u8], right: &[u8]) {
        let mut index = 0;
        while index < left.len() {
            if self.in_region {
                let run = differing_run(&left[index..], &right[index..]);
                self.differing += run as u64;
                index += run;
                if index < left.len() {
                    self.in_region = false;
                }
            } else if let Some(at) = first_difference(&left[index..], &right[index..]) {
                index += at;
                self.start_region(offset + index as u64);
            } else {
                index = left.len();
            }
        }
    }

    /// Bytes only one file has.
    fn mark_tail(&mut self, start: u64, length: u64) {
        if !self.in_region {
            self.start_region(start);
        }
        self.differing += length;
    }

    fn start_region(&mut self, start: u64) {
        self.in_region = true;
        self.regions += 1;
        self.first.get_or_insert(start);
        self.stops.push(start);
    }

    fn total(&self) -> u64 {
        self.lengths[0].max(self.lengths[1])
    }

    fn percent(&self) -> u64 {
        let common = self.lengths[0].min(self.lengths[1]);
        if common == 0 {
            100
        } else {
            (u128::from(self.offset.min(common)) * 100 / u128::from(common)) as u64
        }
    }
}

/// Offset of the first differing byte, comparing 64-byte chunks before single bytes.
fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    if left == right {
        return None;
    }
    let mut at = 0;
    for (a, b) in left.chunks(64).zip(right.chunks(64)) {
        if a != b {
            return a
                .iter()
                .zip(b)
                .position(|(x, y)| x != y)
                .map(|inner| at + inner);
        }
        at += a.len();
    }
    None
}

/// Length of the run of differing bytes at the start, eight bytes at a time.
fn differing_run(left: &[u8], right: &[u8]) -> usize {
    let mut at = 0;
    for (a, b) in left.as_chunks::<8>().0.iter().zip(right.as_chunks::<8>().0) {
        let same = u64::from_ne_bytes(*a) ^ u64::from_ne_bytes(*b);
        // Some byte of `same` is zero exactly when a byte pair is equal.
        if same.wrapping_sub(0x0101_0101_0101_0101) & !same & 0x8080_8080_8080_8080 != 0 {
            break;
        }
        at += 8;
    }
    at + left[at..]
        .iter()
        .zip(&right[at..])
        .take_while(|(a, b)| a != b)
        .count()
}

fn check_current(cancel: &AtomicBool, is_current: &impl Fn() -> bool) -> Result<(), String> {
    check_cancel(cancel)?;
    if is_current() {
        Ok(())
    } else {
        Err("Comparison was replaced before the byte comparison completed.".into())
    }
}

/// The bytes of both files around the visible rows.
#[derive(Default)]
struct Window {
    start: u64,
    end: u64,
    bytes: [Vec<u8>; 2],
}

impl Window {
    /// The bytes of `side` in `range`, cut at that file's end.
    fn get(&self, side: usize, range: Range<u64>) -> &[u8] {
        let bytes = &self.bytes[side];
        let start = (range.start.saturating_sub(self.start) as usize).min(bytes.len());
        let end = (range.end.saturating_sub(self.start) as usize).clamp(start, bytes.len());
        &bytes[start..end]
    }
}

pub(crate) struct BinaryComparison {
    pub scan: Scan,
    /// Why the pair is shown as bytes when nobody chose it: a refused text comparison or
    /// a pair that is not two images.
    reason: Option<String>,
    ssd: [bool; 2],
    renderer: TextRenderer,
    pub typeface: Typeface,
    /// Offset of the first visible row, a multiple of `row_bytes`.
    top: u64,
    row_bytes: u64,
    pub horizontal: f32,
    vertical_remainder: f32,
    /// Logical row height and fully visible rows of the last painted frame.
    row_height: f32,
    page_rows: u64,
    /// A jump to a difference the scan has not reached yet.
    pending: Option<Jump>,
    window: Window,
    /// The advance of one character for a family, size and scale.
    advance: Option<(String, u32, u32, f32)>,
}

impl BinaryComparison {
    pub fn new(sources: &[CapturedFile; 2], ssd: [bool; 2], reason: Option<String>) -> Self {
        Self {
            scan: Scan::new(sources, ssd),
            reason,
            ssd,
            renderer: TextRenderer::default(),
            typeface: Typeface::default(),
            top: 0,
            row_bytes: ROW_BYTES[0],
            horizontal: 0.0,
            vertical_remainder: 0.0,
            row_height: 24.0,
            page_rows: 1,
            pending: None,
            window: Window::default(),
            advance: None,
        }
    }

    fn row(&self, offset: u64) -> u64 {
        offset / self.row_bytes
    }

    fn last_row(&self) -> u64 {
        self.scan.total().div_ceil(self.row_bytes).saturating_sub(1)
    }

    fn show_row(&mut self, row: u64) {
        self.top = row.min(self.last_row()) * self.row_bytes;
        self.vertical_remainder = 0.0;
    }

    pub fn scroll(&mut self, rows: i64) {
        self.pending = None;
        let row = self.row(self.top).saturating_add_signed(rows);
        self.top = row.min(self.last_row()) * self.row_bytes;
    }

    /// Wheel distances use the painted row height, as in text comparison.
    pub fn scroll_pixels(&mut self, pixels: f32) {
        self.vertical_remainder -= pixels / self.row_height;
        let rows = self.vertical_remainder.trunc();
        self.vertical_remainder -= rows;
        self.scroll(rows as i64);
    }

    pub fn scroll_pages(&mut self, pages: i32) {
        self.vertical_remainder = 0.0;
        self.scroll(i64::from(pages).saturating_mul(self.page_rows as i64));
    }

    pub fn seek(&mut self, amount: f64) {
        if amount.is_finite() {
            self.pending = None;
            let row = (amount.clamp(0.0, 1.0) * self.last_row() as f64) as u64;
            self.show_row(row);
        }
    }

    /// The next (or previous) region on another row, stopping at the first and last one
    /// like text comparison. A region the scan has not reached yet is shown once found.
    pub fn difference(&mut self, backwards: bool) {
        let top = self.row(self.top);
        let stops = &self.scan.stops.starts;
        let target = if backwards {
            let before = stops.partition_point(|&start| start / self.row_bytes < top);
            before
                .checked_sub(1)
                .map(|index| stops[index])
                .or(stops.first().copied())
        } else {
            let after = stops.partition_point(|&start| start / self.row_bytes <= top);
            match stops.get(after) {
                Some(&start) => Some(start),
                None if !self.scan.complete => {
                    self.pending = Some(Jump::After(top));
                    return;
                }
                None => stops.last().copied(),
            }
        };
        self.pending = None;
        if let Some(start) = target {
            self.show_row(self.row(start));
        }
    }

    pub fn difference_edge(&mut self, last: bool) {
        self.pending = Some(if last { Jump::Last } else { Jump::First });
        self.resolve_pending();
    }

    /// Perform a jump that waits for the scan once its target is known. True when the
    /// view or the jump changed.
    pub fn resolve_pending(&mut self) -> bool {
        let Some(jump) = self.pending else {
            return false;
        };
        let stops = &self.scan.stops.starts;
        let target = match jump {
            Jump::First => stops.first().copied(),
            Jump::After(row) => {
                let after = stops.partition_point(|&start| start / self.row_bytes <= row);
                stops
                    .get(after)
                    .copied()
                    .or_else(|| self.scan.complete.then(|| stops.last().copied()).flatten())
            }
            Jump::Last => self.scan.complete.then(|| stops.last().copied()).flatten(),
        };
        if let Some(start) = target {
            self.pending = None;
            self.show_row(self.row(start));
            return true;
        }
        if self.scan.complete {
            self.pending = None;
            return true;
        }
        false
    }

    /// The width of one character, measured once per family, size and scale.
    fn advance(&mut self, options: &TextOptions<'_>) -> Result<f32, String> {
        let key = (options.size.to_bits(), options.scale.to_bits());
        if let Some((family, size, scale, advance)) = &self.advance
            && family == options.family
            && (*size, *scale) == key
        {
            return Ok(*advance);
        }
        let probe = "0123456789ABCDEF";
        let page = self
            .renderer
            .layout_with_options(probe, options, &[])
            .map_err(|error| format!("Cannot lay out byte comparison: {error:?}"))?;
        let advance = (page.width() / probe.len() as f32).max(1.0);
        self.advance = Some((options.family.to_owned(), key.0, key.1, advance));
        Ok(advance)
    }

    /// Read the bytes of both files around `range`, unless the window already has them.
    fn load(
        &mut self,
        sources: &[CapturedFile; 2],
        range: Range<u64>,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let total = self.scan.total();
        if self.window.start <= range.start && range.end.min(total) <= self.window.end {
            return Ok(());
        }
        let page = range.end - range.start;
        let start = range.start.saturating_sub(page);
        let end = range.end.saturating_add(page).min(total);
        let mut bytes: [Vec<u8>; 2] = Default::default();
        for (side, source) in sources.iter().enumerate() {
            let length = source.length();
            let (from, to) = (start.min(length), end.min(length));
            if from == to {
                continue;
            }
            check_cancel(cancel)?;
            let budget = midnight_work_budget::shared_io();
            let _permit = budget
                .acquire(storage_admission(self.ssd[side]), || {
                    cancel.load(std::sync::atomic::Ordering::Acquire)
                })
                .ok_or("Comparison cancelled.")?;
            let block = source
                .read_at(from, (to - from) as u32)
                .map_err(|error| format!("Compared file changed or could not be read: {error}"))?;
            if block.len() as u64 != to - from {
                return Err("Compared file returned a short read.".into());
            }
            bytes[side] = block;
        }
        self.window = Window { start, end, bytes };
        Ok(())
    }

    pub fn paint(
        &mut self,
        geometry: ViewerGeometry,
        appearance: &ViewerAppearance,
        palette: Palette,
        sources: &[CapturedFile; 2],
        cancel: &AtomicBool,
    ) -> Result<Frame, String> {
        let family = self.typeface.painted().to_owned();
        let options = TextOptions {
            family: &family,
            literal_family: family != "monospace",
            size: appearance.font_size,
            scale: geometry.scale,
            foreground: palette.foreground,
            ..TextOptions::default()
        };
        let (width, height) = (geometry.width, geometry.height);
        let mut pixels = palette.background.repeat(width as usize * height as usize);
        let row_height = (appearance.font_size * geometry.scale * 1.6)
            .ceil()
            .max(12.0) as u32;
        let count = height.div_ceil(row_height).min(MAX_ROWS);
        self.row_height = row_height as f32 / geometry.scale;
        self.page_rows = u64::from((height / row_height).max(1));
        let digits = offset_digits(self.scan.total());
        let advance = self.advance(&options)?;
        let pane = (width / 2) as f32;
        let row_bytes = ROW_BYTES
            .into_iter()
            .find(|&bytes| (line_chars(digits, bytes) + 1) as f32 * advance <= pane)
            .unwrap_or(ROW_BYTES[ROW_BYTES.len() - 1]);
        self.row_bytes = row_bytes;
        // Keep the wheel's fractional rows: only align and clamp the first row.
        self.top = self.row(self.top).min(self.last_row()) * row_bytes;
        let visible = self.top..self.top + u64::from(count) * row_bytes;
        self.load(sources, visible.clone(), cancel)?;
        let mut accessible_text = String::new();
        let mut line = String::new();
        let mut highlights = Vec::new();
        let total = self.scan.total();
        for (index, start) in visible.step_by(row_bytes as usize).enumerate() {
            if start >= total {
                break;
            }
            check_cancel(cancel)?;
            let range = start..start + row_bytes;
            let cells = [0, 1].map(|side| self.window.get(side, range.clone()));
            let differs: Vec<bool> = (0..row_bytes as usize)
                .map(|column| cells[0].get(column) != cells[1].get(column))
                .collect();
            let changed = differs.iter().any(|&differs| differs);
            let y = index as u32 * row_height;
            let rows = row_height.min(height - y);
            for (side, bytes) in cells.into_iter().enumerate() {
                line.clear();
                highlights.clear();
                if !bytes.is_empty() {
                    format_row(start, digits, bytes, &differs, &mut line, &mut highlights);
                }
                accessible_text.push_str(&line);
                accessible_text.push(if side == 0 { '\t' } else { '\n' });
                let left = if side == 0 { 0 } else { width / 2 };
                let side_width = if side == 0 {
                    width / 2
                } else {
                    width - width / 2
                };
                let background = if changed {
                    changed_background(palette.background, side)
                } else {
                    palette.background
                };
                let mut raster = background.repeat(side_width as usize * rows as usize);
                if !line.is_empty() {
                    let page = self
                        .renderer
                        .layout_with_options(&line, &options, &[])
                        .map_err(|error| format!("Cannot lay out byte comparison: {error:?}"))?;
                    let marked = tinted(palette.background, side, 3, 5);
                    for range in &highlights {
                        let rectangles = page
                            .selection_geometry(range.clone())
                            .map_err(|error| format!("Cannot mark differing bytes: {error:?}"))?;
                        for rect in rectangles {
                            let from = (rect.x0 - f64::from(self.horizontal)).floor();
                            let to = (rect.x1 - f64::from(self.horizontal)).ceil();
                            fill_columns(&mut raster, side_width, from, to, marked);
                        }
                    }
                    let mut target = RasterMut::new(side_width, rows, &mut raster)
                        .map_err(|error| format!("Cannot render byte comparison: {error:?}"))?;
                    self.renderer
                        .paint_over_into(&page, &mut target, [self.horizontal, 0.0])
                        .map_err(|error| format!("Cannot render byte comparison: {error:?}"))?;
                }
                for row_y in 0..rows as usize {
                    let source = row_y * side_width as usize * 4;
                    let target = ((y as usize + row_y) * width as usize + left as usize) * 4;
                    pixels[target..target + side_width as usize * 4]
                        .copy_from_slice(&raster[source..source + side_width as usize * 4]);
                }
            }
        }
        let last_row = self.last_row();
        Ok(Frame {
            find: Arc::default(),
            fonts: self.typeface.fonts.clone(),
            #[cfg(test)]
            text_family: Some(family.clone()),
            geometry_revision: 0,
            comparison_generation: 0,
            identity: IdentityVerdict::Pending,
            mode: None,
            width,
            height,
            pixels,
            layers: None,
            accessible_text,
            status: self.status(digits),
            image: false,
            bytes: true,
            bytes_only: false,
            scanning: false,
            position: self.row(self.top) as f64 / last_row.max(1) as f64,
        })
    }

    /// The summary: the counts or progress, the first difference, the sizes, the
    /// position and why bytes are shown.
    fn status(&self, digits: usize) -> String {
        let scan = &self.scan;
        let mut status = String::new();
        let counted = format!(
            "{} differing {} in {} {}",
            grouped(scan.differing),
            if scan.differing == 1 { "byte" } else { "bytes" },
            grouped(scan.regions),
            if scan.regions == 1 {
                "region"
            } else {
                "regions"
            },
        );
        if !scan.complete {
            let _ = write!(status, "Comparing bytes… {}%", scan.percent());
            if scan.differing > 0 {
                let _ = write!(status, " · {counted} so far");
            }
        } else if scan.differing == 0 {
            status.push_str("Bytes identical");
        } else {
            status.push_str(&counted);
        }
        if let Some(first) = scan.first {
            let _ = write!(
                status,
                " · first difference at 0x{first:0digits$X} ({})",
                grouped(first)
            );
        }
        let [left, right] = scan.lengths;
        if left == right {
            let _ = write!(status, " · {} bytes each", grouped(left));
        } else {
            let _ = write!(
                status,
                " · File A {} bytes, File B {} bytes",
                grouped(left),
                grouped(right)
            );
        }
        let _ = write!(status, " · offset 0x{:0digits$X}", self.top);
        if let Some(reason) = &self.reason {
            let _ = write!(status, " · {reason}; compared as bytes");
        }
        if self.pending.is_some() {
            status.push_str(" · finding the next difference…");
        }
        if scan.stops.spacing > 1 {
            let _ = write!(
                status,
                " · regions closer than {} bytes share a stop",
                grouped(scan.stops.spacing)
            );
        }
        status
    }
}

/// Hex digits of the offset column: an even number, at least four.
fn offset_digits(total: u64) -> usize {
    let needed = (64 - total.saturating_sub(1).leading_zeros()).div_ceil(4) as usize;
    needed.max(4).next_multiple_of(2)
}

/// Characters of a row: offset, two spaces, `XX ` per byte, `│ ` and the characters.
fn line_chars(digits: usize, row_bytes: u64) -> usize {
    digits + 4 + 4 * row_bytes as usize
}

/// Format one side of a row of `differs.len()` cells and collect the byte ranges of its
/// differing cells, joining neighbouring cells (and the space between two hex cells).
fn format_row(
    offset: u64,
    digits: usize,
    bytes: &[u8],
    differs: &[bool],
    line: &mut String,
    highlights: &mut Vec<Range<usize>>,
) {
    let _ = write!(line, "{offset:0digits$X}  ");
    let mark =
        |range: Range<usize>, joined: usize, highlights: &mut Vec<Range<usize>>| match highlights
            .last_mut()
        {
            Some(last) if last.end + joined == range.start => last.end = range.end,
            _ => highlights.push(range),
        };
    for (column, &differing) in differs.iter().enumerate() {
        match bytes.get(column) {
            Some(byte) => {
                let at = line.len();
                let _ = write!(line, "{byte:02X} ");
                if differing {
                    mark(at..at + 2, 1, highlights);
                }
            }
            None => line.push_str("   "),
        }
    }
    line.push_str("│ ");
    let hex = highlights.len();
    for (column, &byte) in bytes.iter().enumerate() {
        let at = line.len();
        line.push(if (0x20..0x7f).contains(&byte) {
            char::from(byte)
        } else {
            '·'
        });
        if differs[column] {
            if highlights.len() == hex {
                highlights.push(at..line.len());
            } else {
                mark(at..line.len(), 0, highlights);
            }
        }
    }
}

/// Fill whole columns `from..to` (physical pixels, clipped) of an opaque row raster.
fn fill_columns(raster: &mut [u8], width: u32, from: f64, to: f64, color: [u8; 4]) {
    let from = from.clamp(0.0, f64::from(width)) as usize;
    let to = to.clamp(0.0, f64::from(width)) as usize;
    if from >= to {
        return;
    }
    for row in raster.chunks_exact_mut(width as usize * 4) {
        row[from * 4..to * 4].as_chunks_mut::<4>().0.fill(color);
    }
}

#[cfg(test)]
#[path = "binary_tests.rs"]
mod tests;
