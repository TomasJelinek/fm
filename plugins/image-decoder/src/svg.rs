//! Static SVG without filesystem, network, script or system-font access.
//!
//! Everything resvg draws is drawn: shapes, paths, text, gradients, patterns, markers,
//! clip paths, masks, filters, local `use` and raster images embedded as `data:` URIs.
//! Content that is not drawn is skipped and named in the decoder's note: scripts and
//! event attributes (never run), HTML in `foreignObject` (its `switch` fallback is drawn),
//! and links to other files or URLs (never opened). A document that is not well-formed
//! or exceeds a bound is refused with the exact reason and bound: `document` checks the
//! source, `references` the expansion of references before conversion, and
//! `render_cost` the drawing buffers and work before rendering.
use crate::Decoded;
use image::RgbaImage;
use resvg::{tiny_skia, usvg};
use std::{
    collections::BTreeSet,
    io::{self, Read},
    sync::Arc,
};

mod document;
mod embedded;
mod references;
mod render_cost;

/// Content that is skipped instead of drawn, in the order the note names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Omission {
    /// `image` or `feImage` linking to a file or URL.
    LinkedImages,
    /// `data:` images that are not PNG, JPEG, GIF or WebP, are damaged or exceed a limit.
    EmbeddedImages,
    /// `foreignObject` without a `switch` fallback.
    HtmlContent,
    /// `use` and paint, clip, mask, filter or marker references into other files.
    OtherFiles,
    /// `script` elements and event attributes.
    Scripts,
}

/// Embedded, unchanged OFL-1.1 Noto Sans fonts; see `fonts/README.md`.
const FONTS: [&[u8]; 3] = [
    include_bytes!("../fonts/NotoSans-Regular.ttf"),
    include_bytes!("../fonts/NotoSansArabic.ttf"),
    include_bytes!("../fonts/NotoSansHebrew.ttf"),
];

pub(super) fn decode(reader: impl Read, target: Option<(u32, u32)>) -> io::Result<Decoded> {
    let mut text = document::read(reader)?;
    let document = document::parse(&mut text)?;
    let census = references::check(&document)?;
    let images = embedded::Images::default();
    let tree =
        usvg::Tree::from_xmltree(&document, &options(census.text, &images)).map_err(conversion)?;
    // The XML tree is no longer needed; free it before the pixel buffers exist.
    drop(document);
    let size = tree.size();
    let original = (size.width().ceil() as u32, size.height().ceil() as u32);
    dimensions(original)?;
    let scale = target.map_or(1.0, |(width, height)| {
        (width as f32 / size.width())
            .min(height as f32 / size.height())
            .min(1.0)
    });
    // Floating-point scaling can land just above an integral target (13 -> 7).
    // Ceil preserves coverage, while the explicit cap preserves the host contract.
    let bounds = target.unwrap_or(original);
    let width = ((size.width() * scale).ceil().max(1.0) as u32).min(bounds.0);
    let height = ((size.height() * scale).ceil().max(1.0) as u32).min(bounds.1);
    super::validate_dimensions(width, height, super::MAX_DECODED_BYTES)?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    let retained = census.tree_bytes().saturating_add(images.retained());
    render_cost::check(&tree, transform, (width, height), retained)?;
    let mut pixels = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| document::invalid("SVG pixel allocation failed".into()))?;
    resvg::render(&tree, transform, &mut pixels.as_mut());
    let pixels = RgbaImage::from_raw(width, height, pixels.take_demultiplied())
        .ok_or_else(|| document::invalid("Invalid SVG raster".into()))?;
    let mut omissions = census.omissions;
    omissions.extend(images.omission());
    Ok(Decoded {
        pixels,
        note: note(&omissions),
    })
}

/// Conversion options: no resource directory, no system fonts, embedded fonts only when
/// the document has text (loading them costs time on every decode), and image resolvers
/// that decode `data:` rasters and open nothing else.
fn options(text: bool, images: &embedded::Images) -> usvg::Options<'static> {
    let mut options = usvg::Options {
        resources_dir: None,
        font_family: "Noto Sans".into(),
        image_href_resolver: images.resolver(),
        ..Default::default()
    };
    if text {
        let fonts = options.fontdb_mut();
        for font in FONTS {
            // Borrow the embedded bytes instead of copying 1.4 MiB per decode.
            fonts.load_font_source(usvg::fontdb::Source::Binary(Arc::new(font)));
        }
        fonts.set_serif_family("Noto Sans");
        fonts.set_sans_serif_family("Noto Sans");
        fonts.set_monospace_family("Noto Sans");
        fonts.set_cursive_family("Noto Sans");
        fonts.set_fantasy_family("Noto Sans");
    }
    options
}

fn conversion(error: usvg::Error) -> io::Error {
    match error {
        usvg::Error::InvalidSize => document::statement(
            "This SVG has no usable size: its width, height or viewBox is zero, negative or \
             missing."
                .into(),
        ),
        // The reference pass bounds the expansion far below usvg's own limits.
        _ => document::limit("This SVG exceeds the renderer's structure limits.".into()),
    }
}

/// The original size must fit the decoder's raster limits, even for a small thumbnail.
fn dimensions((width, height): (u32, u32)) -> io::Result<()> {
    if super::validate_dimensions(width, height, super::MAX_DECODED_BYTES).is_ok() {
        return Ok(());
    }
    let edge = super::MAX_DECODED_EDGE;
    Err(document::limit(if width > edge || height > edge {
        format!("This SVG is {width} x {height} pixels; the limit is {edge} pixels per side.")
    } else {
        format!(
            "This SVG is {width} x {height} pixels, more than the limit of {} MiB of pixels.",
            super::MAX_DECODED_BYTES >> 20
        )
    }))
}

/// One line for the status bar: what was not drawn.
fn note(omissions: &BTreeSet<Omission>) -> Option<String> {
    if omissions.is_empty() {
        return None;
    }
    let parts: Vec<&str> = omissions
        .iter()
        .map(|omission| match omission {
            Omission::LinkedImages => "linked images",
            Omission::EmbeddedImages => "embedded images that cannot be decoded",
            Omission::HtmlContent => "HTML content (foreignObject)",
            Omission::OtherFiles => "references to other files",
            Omission::Scripts => "scripts",
        })
        .collect();
    Some(format!("Not shown: {}.", parts.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    /// The expansion and drawing usage of a document at its original size.
    fn usage(svg: &str) -> (references::Census, render_cost::Usage) {
        let mut text = svg.to_owned();
        let document = document::parse(&mut text).unwrap();
        let census = references::check(&document).unwrap();
        let images = embedded::Images::default();
        let tree = usvg::Tree::from_xmltree(&document, &options(census.text, &images)).unwrap();
        let size = (
            tree.size().width().ceil() as u32,
            tree.size().height().ceil() as u32,
        );
        let usage = render_cost::check(
            &tree,
            tiny_skia::Transform::identity(),
            size,
            census.tree_bytes().saturating_add(images.retained()),
        )
        .unwrap();
        (census, usage)
    }

    /// A large exported flowchart: 1,200 rounded boxes with a gradient and a shared drop
    /// shadow filter, two-line labels, arrow connectors with markers, a patterned grid
    /// background and a clipped page group, on a 4000 x 3000 canvas.
    fn flowchart() -> String {
        let mut svg = String::from(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="4000" height="3000" font-family="Segoe UI, Arial, sans-serif" font-size="12"><defs><linearGradient id="fill" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#dae8fc"/></linearGradient><filter id="shadow" x="-10%" y="-10%" width="130%" height="140%"><feGaussianBlur in="SourceAlpha" stdDeviation="2"/><feOffset dx="2" dy="3" result="blur"/><feMerge><feMergeNode in="blur"/><feMergeNode in="SourceGraphic"/></feMerge></filter><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z"/></marker><pattern id="grid" width="20" height="20" patternUnits="userSpaceOnUse"><path d="M20 0H0V20" fill="none" stroke="#eeeeee"/></pattern><clipPath id="page"><rect width="4000" height="3000"/></clipPath></defs><rect width="4000" height="3000" fill="url(#grid)"/><g clip-path="url(#page)">"##,
        );
        for index in 0..1200 {
            let (x, y) = (40 + index % 30 * 130, 40 + index / 30 * 72);
            write!(
                svg,
                r##"<g filter="url(#shadow)"><rect x="{x}" y="{y}" width="100" height="44" rx="6" fill="url(#fill)" stroke="#6c8ebf"/></g><text x="{}" y="{}" text-anchor="middle"><tspan x="{0}">Step {index}</tspan><tspan x="{0}" dy="14">synthetic</tspan></text><path d="M{} {} h10 v36 h-10" fill="none" stroke="#333333" marker-end="url(#arrow)"/>"##,
                x + 50,
                y + 18,
                x + 100,
                y + 22
            )
            .unwrap();
        }
        svg + "</g></svg>"
    }

    /// A detailed map export: 20,000 ten-point polygons in 50 clipped groups, 500 with a
    /// hatch pattern, 2,000 labels and a vignette mask over everything, 3000 x 2000.
    fn map() -> String {
        let mut svg = String::from(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="3000" height="2000" font-family="Arial" font-size="9"><defs><pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><path d="M0 0V6" stroke="#8899aa"/></pattern><radialGradient id="fade"><stop offset="0.7" stop-color="#ffffff"/><stop offset="1" stop-color="#000000"/></radialGradient><mask id="vignette"><rect width="3000" height="2000" fill="url(#fade)"/></mask>"##,
        );
        for group in 0..50 {
            let (x, y) = (group % 10 * 300, group / 10 * 400);
            write!(
                svg,
                r#"<clipPath id="c{group}"><rect x="{x}" y="{y}" width="300" height="400"/></clipPath>"#
            )
            .unwrap();
        }
        svg += r#"</defs><g mask="url(#vignette)">"#;
        for group in 0..50 {
            write!(svg, r#"<g clip-path="url(#c{group})">"#).unwrap();
            for item in 0..400 {
                let (x, y) = (
                    group % 10 * 300 + item % 20 * 15,
                    group / 10 * 400 + item / 20 * 20,
                );
                let fill = if item % 40 == 0 {
                    "url(#hatch)"
                } else {
                    "#c8d8a8"
                };
                write!(
                    svg,
                    r##"<polygon points="{x},{y} {},{y} {},{} {},{} {},{} {x},{} {},{} {},{} {},{} {x},{}" fill="{fill}" stroke="#667755"/>"##,
                    x + 7, x + 14, y + 3, x + 14, y + 9, x + 12, y + 18, y + 18, x + 3, y + 15,
                    x + 1, y + 10, x + 2, y + 6, y + 2
                )
                .unwrap();
                if item % 10 == 0 {
                    write!(svg, r#"<text x="{x}" y="{}">P{item}</text>"#, y + 9).unwrap();
                }
            }
            svg += "</g>";
        }
        svg + "</g></svg>"
    }

    #[test]
    fn large_realistic_documents_stay_far_below_every_bound() {
        for (name, svg) in [("flowchart", flowchart()), ("map", map())] {
            assert!(svg.len() < document::MAX_BYTES as usize, "{name}");
            let (census, usage) = usage(&svg);
            eprintln!(
                "{name}: {} bytes, {} elements and {} data bytes after expansion, peak \
                 buffers {} MiB, work {} M, {} drawn",
                svg.len(),
                census.instances,
                census.data,
                usage.peak >> 20,
                usage.work / 1_000_000,
                usage.drawn
            );
            assert!(
                census.tree_bytes() < references::MAX_TREE_BYTES / 4,
                "{name}"
            );
            assert!(usage.peak < render_cost::MAX_MEMORY_BYTES / 2, "{name}");
            assert!(usage.work < render_cost::MAX_WORK / 4, "{name}");
            assert!(usage.drawn < render_cost::MAX_DRAWN / 4, "{name}");
        }
    }

    #[test]
    fn the_note_names_each_omission_once_in_a_fixed_order_within_a_status_line() {
        assert_eq!(note(&BTreeSet::new()), None);
        let all = BTreeSet::from([
            Omission::Scripts,
            Omission::OtherFiles,
            Omission::HtmlContent,
            Omission::EmbeddedImages,
            Omission::LinkedImages,
        ]);
        let text = note(&all).unwrap();
        assert_eq!(
            text,
            "Not shown: linked images, embedded images that cannot be decoded, HTML content \
             (foreignObject), references to other files, scripts."
        );
        assert!(text.len() <= 160 && text.bytes().all(|byte| (0x20..0x7f).contains(&byte)));
        assert!(!text.contains(['/', '\\']));
    }
}
