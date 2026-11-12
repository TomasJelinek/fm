//! Bounded static artwork rasterization. No external images, fonts, or network access.
use midnight_core::Cancellation;
use midnight_platform_common::icon_snapshot::SnapshotPixels;
use std::collections::{HashMap, HashSet};
const MAX_SVG_BYTES: usize = 128 * 1024;
pub(super) fn decode(bytes: &[u8], size: u16, cancel: &Cancellation) -> Option<SnapshotPixels> {
    if cancel.is_cancelled()
        || !(1..=128).contains(&size)
        || bytes.len() > MAX_SVG_BYTES
        || bytes
            .iter()
            .filter(|byte| **byte == b'<')
            .take(4097)
            .count()
            > 4096
    {
        return None;
    }
    // Parse plain UTF-8 only; do not enable SVGZ decompression or inherited resource
    // authority through optional features unified elsewhere in the application.
    let text = std::str::from_utf8(bytes).ok()?;
    let options = resvg::usvg::Options {
        resources_dir: None,
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        // The default font database is empty; no system fonts are loaded.
        ..Default::default()
    };
    let document = resvg::usvg::roxmltree::Document::parse_with_options(
        text,
        resvg::usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 4096,
            entity_resolver: None,
        },
    )
    .ok()?;
    bounded_expansion(&document, cancel)?;
    let tree = resvg::usvg::Tree::from_xmltree(&document, &options).ok()?;
    if cancel.is_cancelled() {
        return None;
    }
    let extent = tree.size();
    if extent.width() > 1_000_000.0 || extent.height() > 1_000_000.0 {
        return None;
    }
    let edge = f32::from(size);
    let scale = (edge / extent.width()).min(edge / extent.height());
    let transform = resvg::tiny_skia::Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        (edge - extent.width() * scale) / 2.0,
        (edge - extent.height() * scale) / 2.0,
    );
    let mut bitmap = resvg::tiny_skia::Pixmap::new(u32::from(size), u32::from(size))?;
    resvg::render(&tree, transform, &mut bitmap.as_mut());
    if cancel.is_cancelled() {
        return None;
    }
    let rgba = bitmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    SnapshotPixels::new(u32::from(size), u32::from(size), rgba).ok()
}
fn bounded_expansion(
    document: &resvg::usvg::roxmltree::Document<'_>,
    cancel: &Cancellation,
) -> Option<()> {
    let mut ids = HashMap::new();
    for node in document.descendants().filter(|node| node.is_element()) {
        if let Some(id) = node.attribute("id")
            && ids.insert(id.to_owned(), node).is_some()
        {
            return None;
        }
    }
    fn visit<'a, 'input>(
        node: resvg::usvg::roxmltree::Node<'a, 'input>,
        ids: &HashMap<String, resvg::usvg::roxmltree::Node<'a, 'input>>,
        active: &mut HashSet<resvg::usvg::roxmltree::NodeId>,
        remaining: &mut usize,
        depth: usize,
        cancel: &Cancellation,
    ) -> Option<()> {
        if cancel.is_cancelled() || depth > 64 || *remaining == 0 || !active.insert(node.id()) {
            return None;
        }
        *remaining -= 1;
        let svg = node
            .tag_name()
            .namespace()
            .is_none_or(|ns| ns == "http://www.w3.org/2000/svg");
        // These subtree-copying effects need a separate expansion policy; ordinary
        // icon paths, shapes, gradients and budgeted local use references are retained.
        if svg
            && matches!(
                node.tag_name().name(),
                "marker" | "pattern" | "mask" | "clipPath" | "filter"
            )
        {
            return None;
        }
        if svg && node.tag_name().name() == "use" {
            let name = node
                .attributes()
                .find(|attribute| attribute.name() == "href" && attribute.namespace().is_none())
                .or_else(|| {
                    node.attributes().find(|attribute| {
                        attribute.name() == "href"
                            && attribute.namespace() == Some("http://www.w3.org/1999/xlink")
                    })
                })?
                .value()
                .strip_prefix('#')?;
            visit(*ids.get(name)?, ids, active, remaining, depth + 1, cancel)?;
        }
        for child in node.children().filter(|child| child.is_element()) {
            visit(child, ids, active, remaining, depth + 1, cancel)?;
        }
        active.remove(&node.id());
        Some(())
    }
    visit(
        document.root_element(),
        &ids,
        &mut HashSet::new(),
        &mut 4096,
        0,
        cancel,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_svg_keeps_aspect_ratio_and_straight_alpha_in_fixed_output() {
        let icon = decode(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><rect width="20" height="10" fill="red" fill-opacity="0.5"/></svg>"#, 128, &Cancellation::default()).unwrap();
        assert_eq!(
            (icon.width, icon.height, icon.rgba.len()),
            (128, 128, 65536)
        );
        let middle = &icon.rgba[(64 * 128 + 64) * 4..][..4];
        assert_eq!(middle, [255, 0, 0, 128]);
        assert_eq!(&icon.rgba[..4], [0, 0, 0, 0]);
    }
    #[test]
    fn file_images_are_unavailable_and_cancelled_or_oversized_input_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let external = root.path().join("external.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 255, 255]))
            .save(&external)
            .unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 1"><image href="{}" width="1" height="1"/><rect x="1" width="1" height="1" fill="red"/></svg>"#,
            external.display()
        );
        let icon = decode(svg.as_bytes(), 128, &Cancellation::default()).unwrap();
        assert_eq!(&icon.rgba[(64 * 128 + 32) * 4..][..4], [0, 0, 0, 0]);
        assert_eq!(&icon.rgba[(64 * 128 + 96) * 4..][..4], [255, 0, 0, 255]);
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(decode(svg.as_bytes(), 128, &cancel).is_none());
        assert!(
            decode(
                &vec![b' '; MAX_SVG_BYTES + 1],
                128,
                &Cancellation::default()
            )
            .is_none()
        );
        assert!(decode(svg.as_bytes(), 129, &Cancellation::default()).is_none());
    }
    #[test]
    fn xml_depth_dtd_and_reference_expansion_are_bounded_before_svg_conversion() {
        let cancel = Cancellation::default();
        let simple = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><defs><rect id="shape" width="1" height="1" fill="red"/></defs><use href="#shape"/></svg>"##;
        assert!(decode(simple, 128, &cancel).is_some());
        let mut input = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><defs><g id="n0"><rect width="1" height="1"/></g>"#,
        );
        for level in 1..20 {
            input.push_str(&format!(
                r##"<g id="n{level}"><use href="#n{}"/><use href="#n{}"/></g>"##,
                level - 1,
                level - 1
            ));
        }
        input.push_str(r##"</defs><use href="#n19"/></svg>"##);
        assert!(decode(input.as_bytes(), 128, &cancel).is_none());
        let no_namespace = input.replace(" xmlns=\"http://www.w3.org/2000/svg\"", "");
        assert!(decode(no_namespace.as_bytes(), 128, &cancel).is_none());
        let mixed_href = input
            .replace(
                "<svg ",
                "<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\" ",
            )
            .replace(
                "<use href=\"#n19\"/>",
                "<use xlink:href=\"#n0\" href=\"#n19\"/>",
            );
        assert!(decode(mixed_href.as_bytes(), 128, &cancel).is_none());
        assert!(decode(br##"<svg viewBox="0 0 1 1"><defs><marker id="m"><path d="M0 0L1 1"/></marker></defs><path marker-mid="url(#m)" d="M0 0L1 1L0 0L1 1"/></svg>"##, 128, &cancel).is_none());
        assert!(decode(br#"<!DOCTYPE svg [<!ENTITY color "red">]><svg xmlns="http://www.w3.org/2000/svg"><rect fill="&color;"/></svg>"#, 128, &cancel).is_none());
        let deep = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
            "<g>".repeat(65),
            "</g>".repeat(65)
        );
        assert!(decode(deep.as_bytes(), 128, &cancel).is_none());
    }
}
