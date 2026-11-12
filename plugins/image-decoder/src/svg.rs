//! Static SVG without source-selected filesystem, font, image or network authority.
use image::RgbaImage;
use resvg::{tiny_skia, usvg};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Read},
    sync::Arc,
};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_NODES: u32 = 32768;
const MAX_EXPANSION: usize = 65536;

pub(super) fn decode(reader: impl Read, target: Option<(u32, u32)>) -> io::Result<RgbaImage> {
    let mut bytes = Vec::new();
    reader.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("SVG source exceeds 4 MiB budget"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid("SVG must be plain UTF-8"))?;
    let document = usvg::roxmltree::Document::parse_with_options(
        text.trim_start_matches('\u{feff}'),
        usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            entity_resolver: None,
        },
    )
    .map_err(io::Error::other)?;
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || root.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
    {
        return Err(invalid("Expected an SVG document"));
    }
    bounded_expansion(&document)?;
    let mut options = usvg::Options {
        resources_dir: None,
        font_family: "Noto Sans".into(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let fonts = Arc::make_mut(&mut options.fontdb);
    for data in [
        include_bytes!("../fonts/NotoSans-Regular.ttf").as_slice(),
        include_bytes!("../fonts/NotoSansArabic.ttf").as_slice(),
        include_bytes!("../fonts/NotoSansHebrew.ttf").as_slice(),
    ] {
        fonts.load_font_data(data.to_vec());
    }
    fonts.set_serif_family("Noto Sans");
    fonts.set_sans_serif_family("Noto Sans");
    fonts.set_monospace_family("Noto Sans");
    fonts.set_cursive_family("Noto Sans");
    fonts.set_fantasy_family("Noto Sans");
    let tree = usvg::Tree::from_xmltree(&document, &options).map_err(io::Error::other)?;
    let size = tree.size();
    let original = (size.width().ceil() as u32, size.height().ceil() as u32);
    super::validate_dimensions(original.0, original.1)?;
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
    super::validate_dimensions(width, height)?;
    let mut pixels = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| invalid("SVG pixel allocation failed"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixels.as_mut(),
    );
    RgbaImage::from_raw(width, height, pixels.take_demultiplied())
        .ok_or_else(|| invalid("Invalid SVG raster"))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "SVG images, scripts, foreign objects and subtree effects are unsupported",
    )
}

fn bounded_expansion(document: &usvg::roxmltree::Document<'_>) -> io::Result<()> {
    let mut ids = HashMap::new();
    for node in document.descendants().filter(|node| node.is_element()) {
        if let Some(id) = node.attribute("id")
            && ids.insert(id, node).is_some()
        {
            return Err(invalid("Duplicate SVG identifier"));
        }
    }
    fn visit<'a, 'input>(
        node: usvg::roxmltree::Node<'a, 'input>,
        ids: &HashMap<&'a str, usvg::roxmltree::Node<'a, 'input>>,
        active: &mut HashSet<usvg::roxmltree::NodeId>,
        remaining: &mut usize,
        depth: usize,
    ) -> io::Result<()> {
        if depth > 64 || *remaining == 0 || !active.insert(node.id()) {
            return Err(invalid("SVG nesting or reference expansion exceeds budget"));
        }
        *remaining -= 1;
        let svg = node.tag_name().namespace().is_none_or(|namespace| {
            namespace.is_empty() || namespace == "http://www.w3.org/2000/svg"
        });
        if svg
            && matches!(
                node.tag_name().name(),
                "script"
                    | "foreignObject"
                    | "image"
                    | "marker"
                    | "pattern"
                    | "mask"
                    | "clipPath"
                    | "filter"
            )
        {
            return Err(unsupported());
        }
        if svg {
            for attribute in node.attributes() {
                if attribute.name().starts_with("on") {
                    return Err(unsupported());
                }
            }
            if matches!(
                node.tag_name().name(),
                "use" | "linearGradient" | "radialGradient"
            ) {
                let href = node
                    .attribute("href")
                    .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")));
                if let Some(href) = href {
                    let id = href.strip_prefix('#').ok_or_else(unsupported)?;
                    let referenced = ids
                        .get(id)
                        .ok_or_else(|| invalid("Missing SVG local reference"))?;
                    visit(*referenced, ids, active, remaining, depth + 1)?;
                }
            }
        }
        for child in node.children().filter(|node| node.is_element()) {
            visit(child, ids, active, remaining, depth + 1)?;
        }
        active.remove(&node.id());
        Ok(())
    }
    let mut remaining = MAX_EXPANSION;
    visit(
        document.root_element(),
        &ids,
        &mut HashSet::new(),
        &mut remaining,
        0,
    )
}
