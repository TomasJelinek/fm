use crate::{FontChoices, Frame, Palette, check_cancel, identity::DecodedText};
use midnight_core::{ViewerAppearance, ViewerGeometry};
use midnight_plugin_host::CapturedFile;
use midnight_viewer_render::{TextOptions, TextPage, TextRenderer, Viewport};
use similar::{DiffTag, TextDiff};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[path = "text_find.rs"]
mod find;

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LINES: usize = 100_000;
const MAX_LINE_BYTES: usize = 16 * 1024;

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

pub(crate) struct TextInput {
    pub text: String,
    pub encoding: TextEncoding,
}

#[derive(Debug, PartialEq, Eq)]
struct Row {
    left: Option<usize>,
    right: Option<usize>,
    changed: bool,
}

pub(crate) struct TextComparison {
    find: find::TextFind,
    lines: [Vec<String>; 2],
    rows: Vec<Row>,
    differences: Vec<usize>,
    renderer: TextRenderer,
    /// The chooser's catalog and selection; `None` until the catalog arrives, which keeps
    /// the chooser disabled.
    fonts: Option<FontChoices>,
    /// The family painted while `fonts` is `None`.
    family: String,
    pub top: usize,
    pub horizontal: f32,
    vertical_remainder: f32,
    /// Logical row height and fully visible rows of the last painted frame.
    row_height: f32,
    page_rows: usize,
    painted_width: u32,
    decoded_identity: DecodedText,
    pub encodings: Option<[TextEncoding; 2]>,
}

/// Each 256 KiB block is admitted by the storage policy: shared on a confirmed local SSD,
/// exclusive otherwise, so a long copy cannot hold back comparisons of SSD files.
pub(crate) fn read(
    file: &CapturedFile,
    confirmed_ssd: bool,
    cancel: &AtomicBool,
) -> Result<TextInput, String> {
    if file.length() > MAX_BYTES {
        return Err("Text comparison supports at most 4 MiB per file.".into());
    }
    let mut bytes = Vec::with_capacity(file.length() as usize);
    while (bytes.len() as u64) < file.length() {
        check_cancel(cancel)?;
        let budget = midnight_work_budget::shared_io();
        let _permit = budget
            .acquire(
                midnight_work_budget::storage_admission(confirmed_ssd),
                || cancel.load(std::sync::atomic::Ordering::Acquire),
            )
            .ok_or("Comparison cancelled.")?;
        let amount = (file.length() - bytes.len() as u64).min(256 * 1024) as u32;
        let block = file
            .read_at(bytes.len() as u64, amount)
            .map_err(|error| error.to_string())?;
        if block.len() != amount as usize {
            return Err("Compared file returned a short read.".into());
        }
        bytes.extend(block);
    }
    file.revalidate().map_err(|error| error.to_string())?;
    let encoding = if bytes.starts_with(&[0xff, 0xfe]) {
        TextEncoding::Utf16LittleBom
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        TextEncoding::Utf16BigBom
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        TextEncoding::Utf8Bom
    } else {
        TextEncoding::Utf8
    };
    Ok(TextInput {
        text: decode(&bytes)?,
        encoding,
    })
}

fn decode(bytes: &[u8]) -> Result<String, String> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err("Invalid UTF-16 text: incomplete code unit.".into());
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
        String::from_utf16(&words).map_err(|_| "Invalid UTF-16 text.")?
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        std::str::from_utf8(bytes).map_err(|_| "Text comparison requires UTF-8 or UTF-16 with a BOM; this file is binary or uses another encoding.")?.to_owned()
    };
    if text.chars().any(|c| c.is_control() && !text_control(c)) {
        return Err("The selected file contains binary control characters.".into());
    }
    if text.split_inclusive('\n').take(MAX_LINES + 1).count() > MAX_LINES {
        return Err("Text comparison supports at most 100,000 lines per file.".into());
    }
    if text
        .split_inclusive('\n')
        .any(|line| line.len() > MAX_LINE_BYTES)
    {
        return Err("Text comparison supports lines up to 16 KiB.".into());
    }
    Ok(text)
}

impl TextComparison {
    pub fn new(left: &str, right: &str, cancel: &AtomicBool) -> Result<Self, String> {
        check_cancel(cancel)?;
        // The deadline bounds the expensive algorithm; on expiry Similar returns a valid,
        // potentially coarser edit sequence, never a claim that unmatched content is equal.
        let diff = TextDiff::configure()
            .timeout(Duration::from_millis(750))
            .diff_lines(left, right);
        check_cancel(cancel)?;
        let lines =
            [left, right].map(|text| text.split_inclusive('\n').map(str::to_owned).collect());
        let mut rows = Vec::new();
        let mut differences = Vec::new();
        for op in diff.ops() {
            check_cancel(cancel)?;
            let changed = op.tag() != DiffTag::Equal;
            if changed {
                differences.push(rows.len());
            }
            let old = op.old_range();
            let new = op.new_range();
            for index in 0..old.len().max(new.len()) {
                rows.push(Row {
                    left: (index < old.len()).then_some(old.start + index),
                    right: (index < new.len()).then_some(new.start + index),
                    changed,
                });
            }
        }
        Ok(Self {
            find: find::TextFind::default(),
            lines,
            rows,
            differences,
            renderer: TextRenderer::default(),
            fonts: None,
            family: "monospace".into(),
            top: 0,
            horizontal: 0.0,
            vertical_remainder: 0.0,
            row_height: 24.0,
            page_rows: 1,
            painted_width: 0,
            decoded_identity: DecodedText::default(),
            encodings: None,
        })
    }

    /// Install the fixed-pitch catalog scanned by the service's font thread and resolve the
    /// preferred family against it. Until then frames carry no font choices and paint in
    /// the family of `set_family`, so the first frame never waits for the font scan.
    pub fn set_font_catalog(&mut self, families: Arc<[String]>, preferred: &str) {
        let selected = resolve_family(&families, preferred);
        self.family.clone_from(&selected);
        self.fonts = Some(FontChoices { families, selected });
    }

    /// The family to paint before the catalog arrives, resolved by a check of at most two
    /// families (`FixedPitchFonts::resolve`).
    pub fn set_family(&mut self, family: String) {
        self.family = family;
    }

    pub fn identity_turn(&mut self) -> Option<bool> {
        self.decoded_identity.step(&self.lines)
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

    pub fn scroll(&mut self, rows: i64) {
        self.top = self
            .top
            .saturating_add_signed(rows as isize)
            .min(self.rows.len().saturating_sub(1));
    }
    /// Wheel distances use the painted row height, so they match the visible rows at
    /// every font size.
    pub fn scroll_pixels(&mut self, pixels: f32) {
        self.vertical_remainder -= pixels / self.row_height;
        let rows = self.vertical_remainder.trunc();
        self.vertical_remainder -= rows;
        self.scroll(rows as i64);
    }
    /// A page moves by the rows that were fully visible, never skipping or repeating one.
    pub fn scroll_pages(&mut self, pages: i32) {
        self.vertical_remainder = 0.0;
        self.scroll(i64::from(pages).saturating_mul(self.page_rows as i64));
    }
    pub fn seek(&mut self, amount: f64) {
        if amount.is_finite() {
            self.top = (amount.clamp(0.0, 1.0) * self.rows.len().saturating_sub(1) as f64) as usize;
        }
    }
    pub fn difference(&mut self, backwards: bool) {
        let target = if backwards {
            self.differences
                .iter()
                .rev()
                .find(|row| **row < self.top)
                .or(self.differences.first())
        } else {
            self.differences
                .iter()
                .find(|row| **row > self.top)
                .or(self.differences.last())
        };
        if let Some(target) = target {
            self.top = *target;
            self.vertical_remainder = 0.0;
        }
    }

    pub fn difference_edge(&mut self, last: bool) {
        if let Some(target) = if last {
            self.differences.last()
        } else {
            self.differences.first()
        } {
            self.top = *target;
            self.vertical_remainder = 0.0;
        }
    }

    pub fn paint(
        &mut self,
        geometry: ViewerGeometry,
        appearance: &ViewerAppearance,
        palette: Palette,
        cancel: &AtomicBool,
    ) -> Result<Frame, String> {
        let family = self
            .fonts
            .as_ref()
            .map_or(self.family.as_str(), |fonts| fonts.selected.as_str());
        let (width, height) = (geometry.width, geometry.height);
        let mut pixels = palette.background.repeat(width as usize * height as usize);
        let row_height = (appearance.font_size * geometry.scale * 1.6)
            .ceil()
            .max(12.0) as u32;
        let count = height.div_ceil(row_height).min(256) as usize;
        self.row_height = row_height as f32 / geometry.scale;
        self.page_rows = (height / row_height).max(1) as usize;
        if self.find.reveal {
            if let Some(hit) = &self.find.selected {
                self.top = hit.row.saturating_sub(self.page_rows / 2);
                self.vertical_remainder = 0.0;
                let row = &self.rows[hit.row];
                let (value, prefix) = self.formatted_line(hit.side, hit.line, row.changed);
                let page = self
                    .renderer
                    .layout_with_options(
                        &value,
                        &TextOptions {
                            family,
                            literal_family: family != "monospace",
                            size: appearance.font_size,
                            scale: geometry.scale,
                            foreground: palette.foreground,
                            ..TextOptions::default()
                        },
                        &[],
                    )
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
        }
        self.painted_width = width;
        let mut accessible_text = String::new();
        for (visible, row) in self.rows.iter().skip(self.top).take(count).enumerate() {
            check_cancel(cancel)?;
            for (side, line) in [row.left, row.right].into_iter().enumerate() {
                let left = if side == 0 { 0 } else { width / 2 };
                let side_width = if side == 0 {
                    width / 2
                } else {
                    width - width / 2
                };
                let (value, prefix) = line
                    .map(|index| self.formatted_line(side, index, row.changed))
                    .unwrap_or_default();
                let selection = self
                    .find
                    .selected
                    .as_ref()
                    .filter(|hit| hit.row == self.top + visible && hit.side == side)
                    .map(|hit| prefix + hit.range.start..prefix + hit.range.end);
                accessible_text.push_str(&value);
                accessible_text.push(if side == 0 { '\t' } else { '\n' });
                let background = if row.changed {
                    let tint = if side == 0 {
                        [180u8, 45, 45, 255]
                    } else {
                        [40u8, 150, 75, 255]
                    };
                    std::array::from_fn(|channel| {
                        if channel == 3 {
                            255
                        } else {
                            ((u16::from(palette.background[channel]) * 3
                                + u16::from(tint[channel]))
                                / 4) as u8
                        }
                    })
                } else {
                    palette.background
                };
                let options = TextOptions {
                    family,
                    literal_family: family != "monospace",
                    size: appearance.font_size,
                    scale: geometry.scale,
                    foreground: palette.foreground,
                    ..TextOptions::default()
                };
                let page = self
                    .renderer
                    .layout_with_options(&value, &options, &[])
                    .map_err(|error| format!("Cannot lay out comparison: {error:?}"))?;
                if let Some(range) = &selection {
                    self.find.horizontal_bounds = Some(shaped_hit_bounds(&page, range.clone())?);
                }
                let raster = self
                    .renderer
                    .rasterize(
                        &page,
                        Viewport {
                            width: side_width,
                            height: row_height.min(height),
                            origin: [self.horizontal, 0.0],
                            background,
                            selection_color: palette.selection,
                        },
                        selection,
                    )
                    .map_err(|error| format!("Cannot render comparison: {error:?}"))?;
                let y = visible as u32 * row_height;
                for row_y in 0..row_height.min(height - y) {
                    let source = row_y as usize * side_width as usize * 4;
                    let target = ((y + row_y) as usize * width as usize + left as usize) * 4;
                    pixels[target..target + side_width as usize * 4]
                        .copy_from_slice(&raster.rgba[source..source + side_width as usize * 4]);
                }
            }
        }
        let status = format!(
            "{} difference groups · row {} of {} · live-validated read",
            self.differences.len(),
            self.top + 1,
            self.rows.len()
        );
        Ok(Frame {
            find: Arc::clone(&self.find.state),
            fonts: self.fonts.clone(),
            #[cfg(test)]
            text_family: Some(family.to_owned()),
            geometry_revision: 0,
            comparison_generation: 0,
            identity: crate::IdentityVerdict::Pending,
            mode: None,
            width,
            height,
            pixels,
            layers: None,
            accessible_text,
            status,
            image: false,
            position: self.top as f64 / self.rows.len().saturating_sub(1).max(1) as f64,
        })
    }

    pub fn change_font(&mut self, family: &str) -> bool {
        let Some(fonts) = self.fonts.as_mut() else {
            return false;
        };
        if fonts.selected == family
            || (family != "monospace" && !fonts.families.iter().any(|name| name == family))
        {
            return false;
        }
        fonts.selected = family.into();
        true
    }

    fn formatted_line(&self, side: usize, index: usize, changed: bool) -> (String, usize) {
        let text = &self.lines[side][index];
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

/// The catalog's name for the preferred family (ASCII case-insensitive), else Consolas,
/// else generic monospace.
pub(crate) fn resolve_family(families: &[String], preferred: &str) -> String {
    if preferred == "monospace" {
        return preferred.into();
    }
    families
        .iter()
        .find(|name| name.eq_ignore_ascii_case(preferred))
        .or_else(|| {
            families
                .iter()
                .find(|name| name.eq_ignore_ascii_case("Consolas"))
        })
        .cloned()
        .unwrap_or_else(|| "monospace".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_choice_wins_and_missing_family_falls_back_to_consolas_then_monospace() {
        let names = ["Cascadia Mono".into(), "Consolas".into()];
        assert_eq!(resolve_family(&names, "Cascadia Mono"), "Cascadia Mono");
        assert_eq!(resolve_family(&names, "consolas"), "Consolas");
        assert_eq!(resolve_family(&names, "Missing Mono"), "Consolas");
        assert_eq!(resolve_family(&names, "monospace"), "monospace");
        assert_eq!(resolve_family(&[], "Consolas"), "monospace");
    }

    #[test]
    fn changing_font_preserves_diff_and_rejects_proportional_or_unknown_families() {
        let mut diff = TextComparison::new("a\nb\n", "A\nB\n", &AtomicBool::new(false)).unwrap();
        diff.fonts = Some(FontChoices {
            families: vec!["Consolas".into(), "Cascadia Mono".into()].into(),
            selected: "Consolas".into(),
        });
        diff.top = 1;
        diff.horizontal = 24.0;
        assert!(!diff.change_font("Arial"));
        assert!(!diff.change_font("Missing Mono"));
        assert!(diff.change_font("Cascadia Mono"));
        assert_eq!(diff.fonts.as_ref().unwrap().selected, "Cascadia Mono");
        assert_eq!(diff.top, 1);
        assert_eq!(diff.horizontal, 24.0);
        assert_eq!(diff.rows.len(), 2);
        assert!(diff.change_font("monospace"));
    }
    #[test]
    fn aligns_insertions_and_replacements_without_losing_unicode() {
        let cancel = AtomicBool::new(false);
        let diff = TextComparison::new(
            "hello\nčeský 🐈\nlast\n",
            "hello\n世界\nčeský 🐈\nLAST\n",
            &cancel,
        )
        .unwrap();
        assert_eq!(diff.rows.iter().filter(|row| row.changed).count(), 2);
        assert!(
            diff.rows
                .iter()
                .any(|row| row.left == Some(1) && row.right == Some(2) && !row.changed)
        );
        assert_eq!(diff.differences.len(), 2);
    }
    #[test]
    fn empty_and_missing_newline_are_distinct() {
        let cancel = AtomicBool::new(false);
        assert!(
            TextComparison::new("", "", &cancel)
                .unwrap()
                .rows
                .is_empty()
        );
        assert_eq!(
            TextComparison::new("a\n", "a", &cancel)
                .unwrap()
                .differences
                .len(),
            1
        );
    }
    #[test]
    fn form_feed_vertical_tab_and_ansi_escape_are_text_shown_as_control_pictures() {
        let text = "page 1\u{0c}\n\u{1b}[31mred\u{1b}[0m\u{0b}x\n";
        assert_eq!(decode(text.as_bytes()).unwrap(), text);
        for binary in ["a\u{1}b", "a\u{7f}b", "a\u{85}b"] {
            assert!(decode(binary.as_bytes()).is_err(), "{binary:?}");
        }
        assert_eq!(visible_controls("a\u{0c}b\u{1b}c\u{0b}"), "a␌b␛c␋");
        assert_eq!(visible_controls("tab\tkept"), "tab\tkept");
        let mut diff = TextComparison::new(text, "page 1\n", &AtomicBool::new(false)).unwrap();
        let geometry = ViewerGeometry {
            width: 400,
            height: 100,
            scale: 1.0,
        };
        let frame = diff
            .paint(
                geometry,
                &appearance(12.0),
                Palette::from(&appearance(12.0)),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(frame.accessible_text.contains("page 1␌"));
        assert!(frame.accessible_text.contains("␛[31mred␛[0m␋x"));
    }

    #[test]
    fn bidi_controls_are_shown_as_escapes() {
        assert_eq!(
            visible_controls("a\u{202e}b\u{2066}c\u{200f}d\u{1b}"),
            "a\\u202Eb\\u2066c\u{200f}d␛"
        );
        for control in ('\u{202a}'..='\u{202e}').chain('\u{2066}'..='\u{2069}') {
            let line = format!("x{control}y");
            assert_eq!(
                visible_controls(&line),
                format!("x\\u{:04X}y", u32::from(control))
            );
        }
        assert!(matches!(
            visible_controls("RTL \u{200e}\u{200f}\u{61c} marks"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn lines_that_differ_only_in_a_bidi_control_show_both_escapes_in_one_changed_row() {
        let cancel = AtomicBool::new(false);
        let mut diff = TextComparison::new("x\u{202e}y\n", "x\u{202c}y\n", &cancel).unwrap();
        assert_eq!(diff.rows.len(), 1);
        assert!(diff.rows[0].changed);
        let geometry = ViewerGeometry {
            width: 400,
            height: 100,
            scale: 1.0,
        };
        let frame = diff
            .paint(
                geometry,
                &appearance(12.0),
                Palette::from(&appearance(12.0)),
                &cancel,
            )
            .unwrap();
        assert!(frame.accessible_text.contains("x\\u202Ey"));
        assert!(frame.accessible_text.contains("x\\u202Cy"));
        assert!(
            !frame
                .accessible_text
                .contains(midnight_core::is_explicit_bidi_control)
        );
    }

    fn appearance(font_size: f32) -> ViewerAppearance {
        ViewerAppearance {
            family: "monospace".into(),
            font_size,
            foreground: [255; 4],
            background: [0, 0, 0, 255],
            selection: [40, 60, 90, 255],
            accent: [90, 150, 220, 255],
        }
    }

    #[test]
    fn pages_advance_by_fully_visible_rows_at_every_font_size() {
        let left: String = (0..400).map(|row| format!("row {row}\n")).collect();
        let cancel = AtomicBool::new(false);
        for (font_size, scale) in [(12.0, 1.0), (15.0, 1.0), (20.0, 1.0), (12.0, 2.0)] {
            let mut diff = TextComparison::new(&left, &left, &cancel).unwrap();
            let height = 300;
            let geometry = ViewerGeometry {
                width: 400,
                height,
                scale,
            };
            diff.paint(
                geometry,
                &appearance(font_size),
                Palette::from(&appearance(font_size)),
                &cancel,
            )
            .unwrap();
            let row_height = (font_size * scale * 1.6).ceil();
            let visible = (height as f32 / row_height).floor() as usize;
            diff.scroll_pages(1);
            assert_eq!(diff.top, visible, "font {font_size} at scale {scale}");
            diff.scroll_pages(-1);
            assert_eq!(diff.top, 0);
            // Wheel pixels are logical; one painted row height scrolls exactly one row.
            diff.scroll_pixels(-(row_height / scale));
            assert_eq!(diff.top, 1, "font {font_size} at scale {scale}");
        }
    }

    #[test]
    fn rejects_binary_invalid_encoding_and_limits() {
        assert!(decode(b"a\0b").is_err());
        assert!(decode(&[0xff]).is_err());
        assert!(decode(&[0xff, 0xfe, 1]).is_err());
        assert!(decode(&vec![b'a'; MAX_LINE_BYTES + 1]).is_err());
        assert_eq!(decode(&[0xff, 0xfe, 0xe9, 0]).unwrap(), "é");
    }
    #[test]
    fn cancellation_and_navigation() {
        assert!(TextComparison::new("a", "b", &AtomicBool::new(true)).is_err());
        let mut diff =
            TextComparison::new("a\nb\nc\n", "A\nb\nC\n", &AtomicBool::new(false)).unwrap();
        diff.difference(false);
        assert_eq!(diff.top, 2);
        diff.difference(true);
        assert_eq!(diff.top, 0);
        diff.seek(f64::NAN);
        assert_eq!(diff.top, 0);
    }
    #[test]
    fn difference_navigation_stops_at_each_boundary_and_refocuses_after_scrolling() {
        let mut diff = TextComparison::new(
            "a\nb\nc\nd\ne\n",
            "A\nb\nC\nd\nE\n",
            &AtomicBool::new(false),
        )
        .unwrap();
        diff.difference(true);
        assert_eq!(diff.top, 0, "Previous must not wrap to the last group");
        diff.difference(false);
        assert_eq!(diff.top, 2);
        diff.difference(false);
        assert_eq!(diff.top, 4);
        diff.difference_edge(false);
        assert_eq!(diff.top, 0);
        diff.difference_edge(true);
        assert_eq!(diff.top, 4);
        diff.difference(false);
        assert_eq!(diff.top, 4, "Next must not wrap to the first group");
        diff.scroll(-1);
        diff.difference(false);
        assert_eq!(diff.top, 4);
        let mut empty = TextComparison::new("", "", &AtomicBool::new(false)).unwrap();
        empty.difference(true);
        empty.difference(false);
        empty.difference_edge(false);
        empty.difference_edge(true);
        assert_eq!(empty.top, 0);
    }
    #[test]
    fn captured_source_change_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("český.txt");
        std::fs::write(&path, "before").unwrap();
        let file = CapturedFile::capture(&path).unwrap();
        std::fs::write(&path, "after mutation").unwrap();
        assert!(read(&file, false, &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn confirmed_ssd_reads_share_the_io_gate_and_unknown_storage_stays_exclusive() {
        use midnight_plugin_host::{ContentSource, Error};
        #[derive(Debug)]
        struct Observed(std::sync::Mutex<Vec<midnight_work_budget::Snapshot>>);
        impl ContentSource for Observed {
            fn length(&self) -> u64 {
                2
            }
            fn display_name(&self) -> String {
                "observed.txt".into()
            }
            fn version(&self) -> Vec<u8> {
                vec![1]
            }
            fn validate(&self) -> Result<(), Error> {
                Ok(())
            }
            fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
                self.0
                    .lock()
                    .unwrap()
                    .push(midnight_work_budget::shared_io().snapshot());
                Ok(b"a\n".to_vec())
            }
        }
        for confirmed_ssd in [true, false] {
            let source = std::sync::Arc::new(Observed(Default::default()));
            let file = CapturedFile::from_source(source.clone()).unwrap();
            assert_eq!(
                read(&file, confirmed_ssd, &AtomicBool::new(false))
                    .unwrap()
                    .text,
                "a\n"
            );
            let snapshot = source.0.lock().unwrap()[0];
            if confirmed_ssd {
                assert!(!snapshot.exclusive && snapshot.active_shared >= 1);
            } else {
                assert!(snapshot.exclusive);
            }
        }
    }
}
