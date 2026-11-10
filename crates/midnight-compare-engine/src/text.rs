use crate::{FontChoices, Frame, check_cancel};
use midnight_core::{ViewerAppearance, ViewerGeometry};
use midnight_plugin_host::CapturedFile;
use midnight_viewer_render::{TextOptions, TextRenderer, Viewport};
use similar::{DiffTag, TextDiff};
use std::{sync::atomic::AtomicBool, time::Duration};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LINES: usize = 100_000;
const MAX_LINE_BYTES: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Row {
    left: Option<usize>,
    right: Option<usize>,
    changed: bool,
}

pub(crate) struct TextComparison {
    lines: [Vec<String>; 2],
    rows: Vec<Row>,
    differences: Vec<usize>,
    renderer: TextRenderer,
    fonts: Option<FontChoices>,
    pub top: usize,
    pub horizontal: f32,
    vertical_remainder: f32,
}

pub(crate) fn read(file: &CapturedFile, cancel: &AtomicBool) -> Result<String, String> {
    if file.length() > MAX_BYTES {
        return Err("Text comparison supports at most 4 MiB per file.".into());
    }
    let mut bytes = Vec::with_capacity(file.length() as usize);
    while (bytes.len() as u64) < file.length() {
        check_cancel(cancel)?;
        let budget = midnight_work_budget::shared_io();
        let _permit = budget
            .acquire(midnight_work_budget::storage_cost(false), || {
                cancel.load(std::sync::atomic::Ordering::Acquire)
            })
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
    decode(&bytes)
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
    if text
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
    {
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
            lines,
            rows,
            differences,
            renderer: TextRenderer::default(),
            fonts: None,
            top: 0,
            horizontal: 0.0,
            vertical_remainder: 0.0,
        })
    }

    pub fn scroll(&mut self, rows: i64) {
        self.top = self
            .top
            .saturating_add_signed(rows as isize)
            .min(self.rows.len().saturating_sub(1));
    }
    pub fn scroll_pixels(&mut self, pixels: f32) {
        self.vertical_remainder -= pixels / 24.0;
        let rows = self.vertical_remainder.trunc();
        self.vertical_remainder -= rows;
        self.scroll(rows as i64);
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
        cancel: &AtomicBool,
    ) -> Result<Frame, String> {
        if self.fonts.is_none() {
            let families = midnight_viewer_render::monospace_families(|| {
                cancel.load(std::sync::atomic::Ordering::Acquire)
            })
            .ok_or("Comparison cancelled.")?;
            let selected = resolve_family(&families, &appearance.family);
            self.fonts = Some(FontChoices { families, selected });
        }
        let family = self
            .fonts
            .as_ref()
            .map(|fonts| fonts.selected.as_str())
            .unwrap_or("monospace");
        let (width, height) = (geometry.width, geometry.height);
        let mut pixels = appearance
            .background
            .repeat(width as usize * height as usize);
        let row_height = (appearance.font_size * geometry.scale * 1.6)
            .ceil()
            .max(12.0) as u32;
        let count = height.div_ceil(row_height).min(256) as usize;
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
                let value = line
                    .map(|index| {
                        let text = &self.lines[side][index];
                        let ending = if text.ends_with("\r\n") {
                            "  [CRLF]"
                        } else if text.ends_with('\n') {
                            ""
                        } else {
                            "  [no newline]"
                        };
                        format!(
                            "{:>6} {} {}{}",
                            index + 1,
                            if row.changed {
                                if side == 0 { "−" } else { "+" }
                            } else {
                                " "
                            },
                            text.trim_end_matches(['\r', '\n']),
                            ending
                        )
                    })
                    .unwrap_or_default();
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
                            ((u16::from(appearance.background[channel]) * 3
                                + u16::from(tint[channel]))
                                / 4) as u8
                        }
                    })
                } else {
                    appearance.background
                };
                let options = TextOptions {
                    family,
                    literal_family: family != "monospace",
                    size: appearance.font_size,
                    scale: geometry.scale,
                    foreground: appearance.foreground,
                    ..TextOptions::default()
                };
                let page = self
                    .renderer
                    .layout_with_options(&value, &options, &[])
                    .map_err(|error| format!("Cannot lay out comparison: {error:?}"))?;
                let raster = self
                    .renderer
                    .rasterize(
                        &page,
                        Viewport {
                            width: side_width,
                            height: row_height.min(height),
                            origin: [self.horizontal, 0.0],
                            background,
                            selection_color: appearance.selection,
                        },
                        None,
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
        let status = if self.differences.is_empty() {
            "Text is identical (encoding and BOM may differ).".into()
        } else {
            format!(
                "{} difference groups · row {} of {} · UTF-8 / UTF-16 · live-validated read",
                self.differences.len(),
                self.top + 1,
                self.rows.len()
            )
        };
        Ok(Frame {
            fonts: self.fonts.clone(),
            geometry_revision: 0,
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
}

fn resolve_family(families: &[String], preferred: &str) -> String {
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
        assert!(read(&file, &AtomicBool::new(false)).is_err());
    }
}
