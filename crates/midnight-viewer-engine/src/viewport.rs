//! Demand for one visible fragment; source offsets preserve overlap when it moves.
use midnight_core::ViewerGeometry;

pub(crate) fn line_budget(geometry: ViewerGeometry, line_height: f32) -> u32 {
    let height = if line_height.is_finite() {
        line_height.max(1.0)
    } else {
        1.0
    };
    // Two potentially clipped edge lines, rather than a file-sized prefetch.
    ((geometry.height as f32 / height).ceil() as u32 + 2).clamp(1, 512)
}

/// A text fragment's final visual line can still gain content. In particular,
/// a terminating newline creates an empty layout row for data not loaded yet.
/// Semantic document windows already retain their complete layout context.
pub(crate) fn available_height(
    page: &crate::page::Page,
    shaped: &midnight_viewer_render::TextPage,
    viewport: f32,
) -> f32 {
    if page.after && page.document_blocks.is_empty() && page.tables.is_empty() {
        shaped
            .line_at_y(shaped.height())
            .map_or(shaped.height(), |(_, top)| {
                if top >= viewport {
                    top
                } else {
                    shaped.height()
                }
            })
    } else {
        shaped.height()
    }
}

pub(crate) struct Continuation {
    pub anchor: u64,
    pub backwards: bool,
    reference: u64,
    offset: f32,
    horizontal: f32,
}

impl Continuation {
    pub fn capture(
        page: &crate::page::Page,
        shaped: &midnight_viewer_render::TextPage,
        geometry: ViewerGeometry,
        origin: [f32; 2],
        delta: f32,
    ) -> Option<Self> {
        let wanted = origin[1] + delta;
        let maximum = (available_height(page, shaped, geometry.height as f32)
            - geometry.height as f32)
            .max(0.0);
        let backwards = if wanted > maximum && page.after {
            false
        } else if wanted < 0.0 && page.before {
            true
        } else {
            return None;
        };
        let at = if backwards {
            wanted + geometry.height as f32
        } else {
            wanted
        };
        let mut continuation = Self::at(page, shaped, [origin[0], wanted], at, backwards)?;
        if page.document_blocks.is_empty()
            && page.tables.is_empty()
            && shaped.height() <= geometry.height as f32
        {
            if !backwards && continuation.anchor <= page.start && page.end > page.start {
                continuation.anchor = page.end;
            } else if backwards && continuation.anchor >= page.end && page.end > page.start {
                continuation.anchor = page.start;
            }
        }
        Some(continuation)
    }

    pub fn refill(
        page: &crate::page::Page,
        shaped: &midnight_viewer_render::TextPage,
        origin: [f32; 2],
    ) -> Option<Self> {
        Self::at(page, shaped, origin, origin[1], false)
    }

    fn at(
        page: &crate::page::Page,
        shaped: &midnight_viewer_render::TextPage,
        origin: [f32; 2],
        at: f32,
        backwards: bool,
    ) -> Option<Self> {
        let (line, top) = shaped.line_at_y(at).ok()?;
        // Binary address columns and document list prefixes have no source
        // boundary at visual x=0. Use the next authoritative anchor, including
        // the next row's first byte for a backwards exclusive endpoint.
        let source = |index| {
            page.anchors
                .iter()
                .find(|a| a.text_offset as usize >= index)
                .or_else(|| page.anchors.last())
        };
        let mut anchor = source(if backwards { line.end } else { line.start })?.source_offset;
        let reference_anchor = source(line.start)?;
        let reference = reference_anchor.source_offset;
        let caret = shaped
            .caret_geometry(reference_anchor.text_offset as usize)
            .ok()?;
        if !backwards && top > at {
            // A semantic crop starting at the next block would discard the
            // visible margin before it; restoring then clamps a negative origin.
            // Retain preceding context using an interior anchor, since the
            // provider's forward crop excludes a block ending at the anchor.
            if let Some(context) = page.document_blocks.iter().rev().find_map(|block| {
                if block.range.end > line.start {
                    return None;
                }
                page.anchors.iter().rev().find(|anchor| {
                    let index = anchor.text_offset as usize;
                    index >= block.range.start
                        && (index < block.range.end
                            || (block.kind == midnight_viewer_render::DocumentKind::Rule
                                && index == block.range.end))
                })
            }) {
                anchor = context.source_offset;
            }
        } else if backwards && caret.y1 < f64::from(at) {
            // The symmetric trailing margin needs the next block. A backward
            // crop excludes a block starting exactly at the requested anchor.
            if let Some(context) = page.document_blocks.iter().find_map(|block| {
                if block.range.start < line.end {
                    return None;
                }
                if block.kind == midnight_viewer_render::DocumentKind::Rule {
                    // A rule's displayed prefix is synthetic and its only
                    // source anchor is the zero-length delimiter start. Use
                    // the following exact boundary, including EOF when final.
                    let rule = page
                        .anchors
                        .iter()
                        .find(|anchor| anchor.text_offset as usize == block.range.end)?;
                    return page
                        .anchors
                        .iter()
                        .find(|anchor| {
                            anchor.text_offset as usize > block.range.end
                                && anchor.source_offset > rule.source_offset
                        })
                        .map(|anchor| anchor.source_offset)
                        .or_else(|| (page.end > rule.source_offset).then_some(page.end));
                }
                let first_source = page
                    .anchors
                    .iter()
                    .find(|anchor| {
                        let index = anchor.text_offset as usize;
                        index >= block.range.start && index <= block.range.end
                    })?
                    .source_offset;
                page.anchors
                    .iter()
                    .find(|anchor| {
                        let index = anchor.text_offset as usize;
                        index >= block.range.start
                            && index <= block.range.end
                            && anchor.source_offset > first_source
                    })
                    .map(|anchor| anchor.source_offset)
            }) {
                anchor = context;
            }
        }
        Some(Self {
            anchor,
            backwards,
            reference,
            offset: origin[1] - caret.y0 as f32,
            horizontal: origin[0],
        })
    }

    pub fn restore(
        self,
        page: &crate::page::Page,
        shaped: &midnight_viewer_render::TextPage,
    ) -> Option<[f32; 2]> {
        let index = page
            .anchors
            .iter()
            .find(|a| a.source_offset == self.reference)?
            .text_offset;
        let caret = shaped.caret_geometry(index as usize).ok()?;
        Some([self.horizontal, caret.y0 as f32 + self.offset])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Page;
    use midnight_plugin_host::api::viewer::Anchor;
    use midnight_viewer_render::TextRenderer;

    fn fragment(text: &str, start: usize, end: usize) -> Page {
        let text = text[start..end].to_owned();
        Page {
            anchors: text
                .char_indices()
                .map(|(at, _)| at)
                .chain(std::iter::once(text.len()))
                .map(|at| Anchor {
                    text_offset: at as u32,
                    source_offset: (start + at) as u64,
                })
                .collect(),
            text,
            spans: vec![],
            styles: vec![],
            tables: vec![],
            document_blocks: Vec::new(),
            style_size: 14.0,
            start: start as u64,
            end: end as u64,
            before: start != 0,
            after: true,
        }
    }

    fn document_fragment(selected: std::ops::Range<usize>) -> Page {
        use midnight_viewer_render::{DocumentBlock, DocumentKind};
        let contents = [
            ("Earlier café paragraph", DocumentKind::Paragraph),
            ("First heading", DocumentKind::Heading(2)),
            ("Body paragraph", DocumentKind::Paragraph),
            ("Next heading", DocumentKind::Heading(2)),
            ("Following paragraph", DocumentKind::Paragraph),
        ];
        let mut page = fragment("", 0, 0);
        page.start = 100 + selected.start as u64 * 100;
        page.end = 100 + selected.end as u64 * 100;
        page.before = true;
        page.anchors.clear();
        for index in selected {
            if !page.text.is_empty() {
                page.text.push_str("\n\n");
            }
            let (content, kind) = contents[index];
            let start = page.text.len();
            page.text.push_str(content);
            page.document_blocks.push(DocumentBlock {
                range: start..page.text.len(),
                kind,
                depth: 0,
            });
            // Artificial block separators have no authoritative source anchors.
            page.anchors.extend(
                content
                    .char_indices()
                    .map(|(at, _)| at)
                    .chain(std::iter::once(content.len()))
                    .map(|at| Anchor {
                        text_offset: (start + at) as u32,
                        source_offset: 100 + index as u64 * 100 + at as u64,
                    }),
            );
        }
        page
    }

    fn document_layout(
        page: &Page,
        renderer: &mut TextRenderer,
    ) -> midnight_viewer_render::TextPage {
        page.layout(
            renderer,
            &midnight_viewer_render::TextOptions {
                family: "system-ui",
                size: 16.0,
                line_height: Some(1.5),
                wrap_width: Some(400.0),
                ..Default::default()
            },
            true,
            [255; 4],
        )
        .unwrap()
    }

    #[test]
    fn forward_continuation_retains_context_before_a_heading_gap() {
        let mut renderer = TextRenderer::default();
        let old = document_fragment(0..3);
        let shaped = document_layout(&old, &mut renderer);
        let heading = old.document_blocks[1].range.start;
        let wanted = shaped.caret_geometry(heading).unwrap().y0 as f32 - 6.0;
        let geometry = ViewerGeometry {
            width: 400,
            height: 100,
            scale: 1.0,
        };
        let pending = Continuation::capture(&old, &shaped, geometry, [0.0; 2], wanted).unwrap();
        let reference = pending.reference;
        let full = document_fragment(0..5);
        // Match semantic_window's strict forward end > anchor crop boundary.
        let first = full
            .document_blocks
            .iter()
            .position(|block| {
                full.anchors
                    .iter()
                    .find(|anchor| anchor.text_offset as usize == block.range.end)
                    .unwrap()
                    .source_offset
                    > pending.anchor
            })
            .unwrap();
        let new = document_fragment(first..5);
        let layout = document_layout(&new, &mut renderer);
        let restored = pending.restore(&new, &layout).unwrap()[1];
        let clamped = restored.clamp(0.0, layout.height() - geometry.height as f32);
        let index = new
            .anchors
            .iter()
            .find(|anchor| anchor.source_offset == reference)
            .unwrap()
            .text_offset as usize;
        let screen_y = layout.caret_geometry(index).unwrap().y0 as f32 - clamped;
        assert!(
            (screen_y - 6.0).abs() < 0.01,
            "heading margin was lost: {screen_y}"
        );
        assert_eq!(first, 0, "retain the preceding complete block");
    }

    #[test]
    fn backward_continuation_retains_context_after_a_paragraph_gap() {
        let mut renderer = TextRenderer::default();
        let old = document_fragment(1..4);
        let shaped = document_layout(&old, &mut renderer);
        let paragraph = old.document_blocks[1].range.start;
        let caret = shaped.caret_geometry(paragraph).unwrap();
        let geometry = ViewerGeometry {
            width: 400,
            height: 100,
            scale: 1.0,
        };
        let wanted = caret.y1 as f32 + 6.0 - geometry.height as f32;
        let pending = Continuation::capture(&old, &shaped, geometry, [0.0; 2], wanted).unwrap();
        let reference = pending.reference;
        let full = document_fragment(0..5);
        // Match semantic_window's strict backward start < anchor crop boundary.
        let last = full
            .document_blocks
            .iter()
            .rposition(|block| {
                full.anchors
                    .iter()
                    .find(|anchor| anchor.text_offset as usize == block.range.start)
                    .unwrap()
                    .source_offset
                    < pending.anchor
            })
            .unwrap();
        let new = document_fragment(0..last + 1);
        let layout = document_layout(&new, &mut renderer);
        let restored = pending.restore(&new, &layout).unwrap()[1];
        let clamped = restored.clamp(0.0, layout.height() - geometry.height as f32);
        let index = new
            .anchors
            .iter()
            .find(|anchor| anchor.source_offset == reference)
            .unwrap()
            .text_offset as usize;
        let screen_bottom = layout.caret_geometry(index).unwrap().y1 as f32 - clamped;
        assert!(
            (screen_bottom - (geometry.height as f32 - 6.0)).abs() < 0.01,
            "paragraph margin was lost: {screen_bottom}"
        );
        assert_eq!(last, 3, "retain the following complete block");
    }

    #[test]
    fn backward_gap_retains_prefixed_quote_and_list_blocks_by_source_boundary() {
        use midnight_viewer_render::DocumentKind;
        for (prefix, kind) in [("│ ", DocumentKind::Quote), ("• ", DocumentKind::ListItem)] {
            let mut page = document_fragment(1..4);
            let next = &mut page.document_blocks[2];
            let start = next.range.start;
            page.text.insert_str(start, prefix);
            next.range.end += prefix.len();
            next.kind = kind;
            for anchor in &mut page.anchors {
                if anchor.text_offset as usize >= start {
                    anchor.text_offset += prefix.len() as u32;
                }
            }
            let first_source = page
                .anchors
                .iter()
                .find(|anchor| anchor.text_offset as usize >= start)
                .unwrap()
                .source_offset;
            let mut renderer = TextRenderer::default();
            let shaped = document_layout(&page, &mut renderer);
            let caret = shaped
                .caret_geometry(page.document_blocks[1].range.start)
                .unwrap();
            let bottom = caret.y1 as f32 + 6.0;
            let pending =
                Continuation::at(&page, &shaped, [0.0, bottom - 100.0], bottom, true).unwrap();
            assert!(
                pending.anchor > first_source,
                "{kind:?} must survive the provider's strict source-start < anchor crop"
            );
        }
    }

    #[test]
    fn exact_document_line_boundaries_do_not_request_neighboring_context() {
        let page = document_fragment(0..5);
        let mut renderer = TextRenderer::default();
        let shaped = document_layout(&page, &mut renderer);
        let paragraph = page.document_blocks[2].range.start;
        let caret = shaped.caret_geometry(paragraph).unwrap();
        let (line, top) = shaped.line_at_y(caret.y0 as f32).unwrap();
        for (backwards, at, expected_index) in
            [(false, top, line.start), (true, caret.y1 as f32, line.end)]
        {
            let expected = page
                .anchors
                .iter()
                .find(|anchor| anchor.text_offset as usize >= expected_index)
                .unwrap()
                .source_offset;
            let pending = Continuation::at(&page, &shaped, [0.0, at], at, backwards).unwrap();
            assert_eq!(pending.anchor, expected);
        }
    }

    #[test]
    fn gap_context_handles_rules_with_only_a_delimiter_source_anchor() {
        use midnight_viewer_render::DocumentKind;
        for backwards in [false, true] {
            let mut page = document_fragment(if backwards { 0..4 } else { 0..5 });
            let rule_index = if backwards { 3 } else { 2 };
            let rule = &mut page.document_blocks[rule_index];
            rule.kind = DocumentKind::Rule;
            let range = rule.range.clone();
            let source = 100 + rule_index as u64 * 100;
            page.anchors.retain_mut(|anchor| {
                let index = anchor.text_offset as usize;
                if index == range.end {
                    anchor.source_offset = source;
                    true
                } else {
                    !range.contains(&index)
                }
            });
            let mut renderer = TextRenderer::default();
            let shaped = document_layout(&page, &mut renderer);
            let at = if backwards {
                shaped
                    .caret_geometry(page.document_blocks[2].range.start)
                    .unwrap()
                    .y1 as f32
                    + 6.0
            } else {
                shaped
                    .caret_geometry(page.document_blocks[3].range.start)
                    .unwrap()
                    .y0 as f32
                    - 6.0
            };
            let pending = Continuation::at(&page, &shaped, [0.0, at], at, backwards).unwrap();
            assert_eq!(pending.anchor, if backwards { page.end } else { source });
        }
    }

    #[test]
    fn forward_and_backward_demands_overlap_and_preserve_the_same_visible_source_pixels() {
        let text = (0..100)
            .map(|i| format!("{i:03} café العربية 世界\n"))
            .collect::<String>();
        let starts: Vec<_> = text.match_indices('\n').map(|(at, _)| at + 1).collect();
        let old = fragment(&text, starts[19], starts[31]);
        let mut renderer = TextRenderer::default();
        let old_layout = renderer.layout(&old.text, 18.0, 1.0).unwrap();
        let line_height = old_layout
            .caret_geometry(old.text.find('\n').unwrap() + 1)
            .unwrap()
            .y0 as f32;
        let geometry = ViewerGeometry {
            width: 800,
            height: (6.0 * line_height) as u32,
            scale: 1.0,
        };
        for (origin, delta, backwards) in [
            (
                [111.0, old_layout.height() - geometry.height as f32],
                line_height * 2.0,
                false,
            ),
            ([111.0, 0.0], -line_height * 2.0, true),
        ] {
            let pending =
                Continuation::capture(&old, &old_layout, geometry, origin, delta).unwrap();
            assert_eq!(pending.backwards, backwards);
            assert!(
                pending.anchor > old.start && pending.anchor < old.end,
                "use overlap, not the next whole page"
            );
            let reference = pending.reference;
            let old_index = old
                .anchors
                .iter()
                .find(|a| a.source_offset == reference)
                .unwrap()
                .text_offset as usize;
            let expected_screen_y =
                old_layout.caret_geometry(old_index).unwrap().y0 as f32 - origin[1] - delta;
            let (start, end) = if backwards {
                let end = pending.anchor as usize;
                let start = text[..end]
                    .match_indices('\n')
                    .rev()
                    .nth(12)
                    .map_or(0, |(at, _)| at + 1);
                (start, end)
            } else {
                let start = pending.anchor as usize;
                let end = text[start..]
                    .match_indices('\n')
                    .nth(11)
                    .map(|(at, _)| start + at + 1)
                    .unwrap();
                (start, end)
            };
            let new = fragment(&text, start, end);
            let layout = renderer.layout(&new.text, 18.0, 1.0).unwrap();
            let restored = pending.restore(&new, &layout).unwrap();
            let index = new
                .anchors
                .iter()
                .find(|a| a.source_offset == reference)
                .unwrap()
                .text_offset as usize;
            let actual_screen_y = layout.caret_geometry(index).unwrap().y0 as f32 - restored[1];
            assert!((actual_screen_y - expected_screen_y).abs() < 0.01);
            assert_eq!(restored[0], 111.0, "horizontal pan survives paging");
        }
    }

    #[test]
    fn content_demand_tracks_visible_rows_instead_of_a_fixed_file_chunk() {
        let geometry = ViewerGeometry {
            width: 900,
            height: 160,
            scale: 1.0,
        };
        let rows = line_budget(geometry, 20.0);
        assert!(
            (8..=10).contains(&rows),
            "requested {rows} rows for eight visible rows"
        );
        assert_eq!(
            rows,
            line_budget(
                ViewerGeometry {
                    width: 1800,
                    height: 320,
                    scale: 2.0,
                },
                40.0,
            )
        );
        assert!(
            line_budget(
                ViewerGeometry {
                    height: 600,
                    ..geometry
                },
                20.0
            ) > rows
        );
    }
}
