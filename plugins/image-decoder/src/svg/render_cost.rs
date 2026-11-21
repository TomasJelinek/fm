//! Bounded drawing work and buffers, before resvg allocates a pixel.
//!
//! resvg draws an isolated group (opacity, clip path, mask, filter or blend mode) into a
//! layer of the group's device bounds, clamped to five times the canvas per side. A clip
//! path or mask adds buffers of the layer's size, each filter primitive keeps a result of
//! the filter region's size until the filter ends, a pattern draws its tile (the pattern
//! rectangle at device scale) for every shape painted with it, and an image is decoded
//! whenever it is drawn. resvg allocates these buffers without a limit, and an allocation
//! the worker cannot satisfy ends the process. This pass follows resvg's recursion with
//! the same transforms and clamps, and refuses a document whose live buffers or pixel work
//! would exceed the bounds below, so the refusal names its reason instead.
//!
//! Gaussian blur costs the same at any radius from 2 device pixels up: resvg uses box
//! blurs with running sums, and a slower recursive filter for smaller deviations, so a
//! blur weighs a constant per region pixel. Morphology, convolution and turbulence weigh
//! their kernel size or octave count.
use super::document::limit;
use resvg::{
    tiny_skia::{IntRect, NonZeroRect, Rect, Transform},
    usvg::{self, Node, Paint, filter::Kind},
};
use std::io;

/// Most memory while drawing: the converted tree (estimated by the reference pass), the
/// retained embedded images, the canvas, and the layers, masks, filter results, pattern
/// tiles and decoded images alive at once. The workers' limit is 512 MiB.
pub(super) const MAX_MEMORY_BYTES: u64 = 448 * 1024 * 1024;
/// Most weighted pixel operations. One unit is about 3 ns of resvg 0.48.1 release work on
/// the development machine, so the bound is roughly half a minute; heavy real documents
/// stay below a tenth of it.
pub(super) const MAX_WORK: u64 = 1 << 33;
/// Most nodes drawn, counting shared content (patterns, cached clip paths) per use.
pub(super) const MAX_DRAWN: u64 = 1 << 20;

/// Per-pixel weights, calibrated with micro-benchmarks of resvg 0.48.1 (measured ns per
/// pixel in parentheses): a fill (0.5 opaque, 4 translucent), a pattern fill (8), a layer
/// or buffer copy (about 3 each), a box blur (173) and a recursive blur for deviations
/// under 2 pixels (340), a flood or other simple primitive (17), lighting (58), a
/// turbulence octave (113), a convolution or morphology kernel element (9 and 4), a
/// decoded image pixel (7) and a stroke dash (315 per dash).
const COPY: u64 = 2;
const PATTERN: u64 = 3;
const BOX_BLUR: u64 = 56;
const RECURSIVE_BLUR: u64 = 110;
const PRIMITIVE: u64 = 6;
const LIGHTING: u64 = 24;
const OCTAVE: u64 = 36;
const KERNEL: u64 = 3;
const DECODE: u64 = 3;
const DASH: u64 = 100;

/// What drawing needs at most.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "tests compare it with the bounds")
)]
pub(super) struct Usage {
    /// Bytes of memory alive at once, including the retained baseline.
    pub(super) peak: u64,
    pub(super) work: u64,
    pub(super) drawn: u64,
}

/// Check drawing `tree` with `transform` onto a canvas of `size`, while `retained` bytes
/// (the converted tree and embedded images) stay in memory.
pub(super) fn check(
    tree: &usvg::Tree,
    transform: Transform,
    size: (u32, u32),
    retained: u64,
) -> io::Result<Usage> {
    let mut cost = Cost {
        live: 0,
        peak: 0,
        work: 0,
        drawn: 0,
        max_bbox: max_bbox(size.0, size.1).ok_or_else(|| {
            limit("This SVG is too large to draw, the limit for SVG images.".into())
        })?,
    };
    cost.allocate(bytes(size).saturating_add(retained))?;
    cost.work(area(size).saturating_mul(COPY))?;
    cost.nodes(tree.root(), transform, size)?;
    Ok(Usage {
        peak: cost.peak,
        work: cost.work,
        drawn: cost.drawn,
    })
}

struct Cost {
    live: u64,
    peak: u64,
    work: u64,
    drawn: u64,
    /// resvg's clamp for layers: the canvas widened by twice its size on every side.
    max_bbox: IntRect,
}

impl Cost {
    fn nodes(&mut self, parent: &usvg::Group, ts: Transform, target: (u32, u32)) -> io::Result<()> {
        for node in parent.children() {
            self.node(node, ts, target)?;
        }
        Ok(())
    }

    fn node(&mut self, node: &Node, ts: Transform, target: (u32, u32)) -> io::Result<()> {
        self.draw()?;
        match node {
            Node::Group(group) => self.group(group, ts, target),
            Node::Path(path) => self.path(path, ts, target),
            Node::Image(image) => self.image(image, ts, target),
            Node::Text(text) => self.group(text.flattened(), ts, target),
        }
    }

    fn group(&mut self, group: &usvg::Group, ts: Transform, target: (u32, u32)) -> io::Result<()> {
        let ts = ts.pre_concat(group.transform());
        if !group.should_isolate() {
            return self.nodes(group, ts, target);
        }
        let Some(layer) = group
            .layer_bounding_box()
            .transform(ts)
            .and_then(|bounds| layer(bounds, !group.filters().is_empty(), self.max_bbox))
        else {
            return Ok(());
        };
        let size = (layer.width(), layer.height());
        self.allocate(bytes(size))?;
        self.work(area(size).saturating_mul(COPY))?;
        let inner = Transform::from_translate(-layer.x() as f32, -layer.y() as f32).pre_concat(ts);
        self.nodes(group, inner, size)?;
        for filter in group.filters() {
            self.filter(filter, inner, size)?;
        }
        if let Some(clip) = group.clip_path() {
            self.clip(clip, inner, size)?;
        }
        if let Some(mask) = group.mask() {
            self.mask(mask, inner, size)?;
        }
        self.release(bytes(size));
        Ok(())
    }

    fn path(&mut self, path: &usvg::Path, ts: Transform, target: (u32, u32)) -> io::Result<()> {
        if !path.is_visible() {
            return Ok(());
        }
        if let Some(fill) = path.fill() {
            self.paint(fill.paint(), path.bounding_box(), ts, target)?;
        }
        if let Some(stroke) = path.stroke() {
            self.paint(stroke.paint(), path.stroke_bounding_box(), ts, target)?;
            if let Some(dashes) = stroke.dasharray() {
                self.work(dash_count(path.data(), dashes).saturating_mul(DASH))?;
            }
        }
        Ok(())
    }

    fn paint(
        &mut self,
        paint: &Paint,
        bounds: Rect,
        ts: Transform,
        target: (u32, u32),
    ) -> io::Result<()> {
        let mut weight = 1;
        if let Paint::Pattern(pattern) = paint {
            self.pattern(pattern, ts)?;
            weight = PATTERN;
        }
        self.work(covered(bounds, ts, target).saturating_mul(weight))
    }

    /// The tile is drawn for every shape painted with the pattern.
    fn pattern(&mut self, pattern: &usvg::Pattern, ts: Transform) -> io::Result<()> {
        let (sx, sy) = ts.pre_concat(pattern.transform()).get_scale();
        let rect = pattern.rect();
        let (width, height) = ((rect.width() * sx).round(), (rect.height() * sy).round());
        // tiny-skia refuses empty tiles and rows longer than i32::MAX bytes: resvg then
        // skips the pattern without allocating.
        let drawn = (1.0..=(i32::MAX / 4) as f32).contains(&width) && height >= 1.0;
        if !drawn {
            return Ok(());
        }
        let size = (width as u32, height.min(u32::MAX as f32) as u32);
        self.allocate(bytes(size))?;
        self.work(area(size))?;
        self.nodes(pattern.root(), Transform::from_scale(sx, sy), size)?;
        self.release(bytes(size));
        Ok(())
    }

    fn image(&mut self, image: &usvg::Image, ts: Transform, target: (u32, u32)) -> io::Result<()> {
        if !image.is_visible() {
            return Ok(());
        }
        if let usvg::ImageKind::SVG(tree) = image.kind() {
            // The decoder never produces nested trees; bound them like a layer anyway.
            self.allocate(bytes(target))?;
            self.work(area(target).saturating_mul(COPY))?;
            self.nodes(tree.root(), ts, target)?;
            self.release(bytes(target));
            return Ok(());
        }
        // resvg decodes the image every time it draws it.
        let decoded = (
            image.size().width().ceil() as u32,
            image.size().height().ceil() as u32,
        );
        self.allocate(bytes(decoded))?;
        self.work(area(decoded).saturating_mul(DECODE))?;
        self.work(covered(image.bounding_box(), ts, target))?;
        self.release(bytes(decoded));
        Ok(())
    }

    /// A clip path draws its content into a buffer of the layer's size, applies its own
    /// clip path, and converts the buffer to a mask.
    fn clip(&mut self, clip: &usvg::ClipPath, ts: Transform, target: (u32, u32)) -> io::Result<()> {
        self.allocate(bytes(target))?;
        self.work(area(target).saturating_mul(3))?;
        self.clip_content(clip.root(), ts.pre_concat(clip.transform()), target)?;
        if let Some(inner) = clip.clip_path() {
            self.clip(inner, ts, target)?;
        }
        self.allocate(area(target))?;
        self.release(area(target));
        self.release(bytes(target));
        Ok(())
    }

    fn clip_content(
        &mut self,
        parent: &usvg::Group,
        ts: Transform,
        target: (u32, u32),
    ) -> io::Result<()> {
        for node in parent.children() {
            self.draw()?;
            match node {
                Node::Path(path) => {
                    if path.is_visible()
                        && let Some(fill) = path.fill()
                    {
                        self.paint(fill.paint(), path.bounding_box(), ts, target)?;
                    }
                }
                Node::Text(text) => self.clip_content(text.flattened(), ts, target)?,
                Node::Group(group) => {
                    let ts = ts.pre_concat(group.transform());
                    match group.clip_path() {
                        // A clipped child is drawn on its own buffer first.
                        Some(clip) => {
                            self.allocate(bytes(target))?;
                            self.work(area(target).saturating_mul(COPY))?;
                            self.clip_content(group, ts, target)?;
                            self.clip(clip, ts, target)?;
                            self.release(bytes(target));
                        }
                        None => self.clip_content(group, ts, target)?,
                    }
                }
                Node::Image(_) => {}
            }
        }
        Ok(())
    }

    /// A mask draws its content into a buffer and an alpha mask of the layer's size.
    fn mask(&mut self, mask: &usvg::Mask, ts: Transform, target: (u32, u32)) -> io::Result<()> {
        if mask.root().children().is_empty() {
            return self.work(area(target));
        }
        let buffers = bytes(target).saturating_add(area(target));
        self.allocate(buffers)?;
        self.work(area(target).saturating_mul(4))?;
        self.nodes(mask.root(), ts, target)?;
        if let Some(inner) = mask.mask() {
            self.mask(inner, ts, target)?;
        }
        self.allocate(area(target))?;
        self.release(area(target));
        self.release(buffers);
        Ok(())
    }

    /// Every primitive's result stays alive until the filter ends; inputs are copied.
    fn filter(
        &mut self,
        filter: &usvg::filter::Filter,
        ts: Transform,
        target: (u32, u32),
    ) -> io::Result<()> {
        let Some(region) = filter
            .rect()
            .transform(ts)
            .map(|rect| rect.to_int_rect())
            .and_then(|region| {
                IntRect::from_xywh(0, 0, target.0, target.1)
                    .and_then(|bounds| fit_to_rect(region, bounds))
            })
        else {
            return Ok(());
        };
        let size = (region.width(), region.height());
        let pixels = area(size);
        let mut held = 0u64;
        for primitive in filter.primitives() {
            self.draw()?;
            let weight = match primitive.kind() {
                Kind::GaussianBlur(blur) => {
                    COPY + blur_weight(blur.std_dev_x().get(), blur.std_dev_y().get(), ts)
                }
                Kind::DropShadow(shadow) => {
                    COPY * 3 + blur_weight(shadow.std_dev_x().get(), shadow.std_dev_y().get(), ts)
                }
                Kind::Morphology(morphology) => {
                    // resvg's kernel spans twice the device radius, at most the region.
                    let (sx, sy) = ts.get_scale();
                    let span = |radius: f32, scale: f32, limit: u32| {
                        (((radius * scale).ceil().max(1.0) as u64).saturating_mul(2))
                            .min(u64::from(limit.max(1)))
                    };
                    COPY + span(morphology.radius_x().get(), sx, size.0).saturating_mul(span(
                        morphology.radius_y().get(),
                        sy,
                        size.1,
                    ))
                }
                Kind::ConvolveMatrix(convolve) => {
                    COPY + KERNEL.saturating_mul(
                        u64::from(convolve.matrix().columns())
                            .saturating_mul(u64::from(convolve.matrix().rows())),
                    )
                }
                Kind::Turbulence(turbulence) => {
                    COPY + OCTAVE.saturating_mul(u64::from(turbulence.num_octaves().max(1)))
                }
                Kind::DiffuseLighting(_) | Kind::SpecularLighting(_) => COPY + LIGHTING,
                Kind::Merge(merge) => COPY.saturating_mul(merge.inputs().len() as u64 + 1),
                _ => PRIMITIVE,
            };
            self.allocate(bytes(size))?;
            held = held.saturating_add(bytes(size));
            self.allocate(bytes(size))?;
            self.release(bytes(size));
            self.work(pixels.saturating_mul(weight))?;
            if let Kind::Image(image) = primitive.kind() {
                // resvg draws filter image content with layers clamped to the region.
                let (sx, sy) = ts.get_scale();
                let outer = self.max_bbox;
                self.max_bbox = IntRect::from_xywh(0, 0, size.0, size.1).unwrap_or(outer);
                let drawn = self.nodes(image.root(), Transform::from_scale(sx, sy), size);
                self.max_bbox = outer;
                drawn?;
            }
        }
        self.work(pixels.saturating_mul(COPY))?;
        self.release(held);
        Ok(())
    }

    fn draw(&mut self) -> io::Result<()> {
        self.drawn += 1;
        if self.drawn > MAX_DRAWN {
            return Err(limit(format!(
                "This SVG draws more than {MAX_DRAWN} elements at this size once shared \
                 content is drawn for every use, the limit for SVG images."
            )));
        }
        Ok(())
    }

    fn allocate(&mut self, bytes: u64) -> io::Result<()> {
        self.live = self.live.saturating_add(bytes);
        self.peak = self.peak.max(self.live);
        if self.live > MAX_MEMORY_BYTES {
            return Err(limit(format!(
                "This SVG needs more than {} MiB of memory to draw at this size (layers, \
                 masks, filters, patterns and images), the limit for SVG images.",
                MAX_MEMORY_BYTES >> 20
            )));
        }
        Ok(())
    }

    fn release(&mut self, bytes: u64) {
        self.live = self.live.saturating_sub(bytes);
    }

    fn work(&mut self, units: u64) -> io::Result<()> {
        self.work = self.work.saturating_add(units);
        if self.work > MAX_WORK {
            return Err(limit(format!(
                "This SVG needs more than {:.1} billion pixel operations to draw at this \
                 size (filters, masks, patterns and layers), the limit for SVG images.",
                MAX_WORK as f64 / 1e9
            )));
        }
        Ok(())
    }
}

/// resvg blurs with three box blurs when a device deviation reaches 2 pixels, otherwise with
/// a slower recursive filter; a deviation of zero on both axes leaves the input.
fn blur_weight(x: f32, y: f32, ts: Transform) -> u64 {
    let (sx, sy) = ts.get_scale();
    let (x, y) = (x * sx, y * sy);
    if x >= 2.0 || y >= 2.0 {
        BOX_BLUR
    } else if x > 0.0 || y > 0.0 {
        RECURSIVE_BLUR
    } else {
        0
    }
}

/// tiny-skia's dash count for a path: its length times the dashes per interval, at most
/// the million it allows. The length of a curve is bounded by its control polygon.
fn dash_count(path: &resvg::tiny_skia::Path, dashes: &[f32]) -> u64 {
    use resvg::tiny_skia::{PathSegment, Point};
    let interval: f32 = dashes.iter().sum();
    if interval.is_nan() || interval <= 0.0 {
        return 0;
    }
    let (mut length, mut start, mut current) = (0.0f32, Point::zero(), Point::zero());
    let mut to = |point: Point, current: &mut Point| {
        length += current.distance(point);
        *current = point;
    };
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(point) => (start, current) = (point, point),
            PathSegment::LineTo(point) => to(point, &mut current),
            PathSegment::QuadTo(control, point) => {
                to(control, &mut current);
                to(point, &mut current);
            }
            PathSegment::CubicTo(first, second, point) => {
                to(first, &mut current);
                to(second, &mut current);
                to(point, &mut current);
            }
            PathSegment::Close => to(start, &mut current),
        }
    }
    let count = length * (dashes.len() / 2) as f32 / interval;
    if count.is_nan() {
        return 0;
    }
    count.min(1_000_000.0).ceil() as u64
}

fn area((width, height): (u32, u32)) -> u64 {
    u64::from(width) * u64::from(height)
}

fn bytes(size: (u32, u32)) -> u64 {
    area(size).saturating_mul(4)
}

/// Device pixels of `bounds` inside the target.
fn covered(bounds: Rect, ts: Transform, (width, height): (u32, u32)) -> u64 {
    let Some(bounds) = bounds.transform(ts) else {
        return 0;
    };
    let horizontal = (bounds.right().min(width as f32) - bounds.left().max(0.0)).max(0.0);
    let vertical = (bounds.bottom().min(height as f32) - bounds.top().max(0.0)).max(0.0);
    (horizontal.ceil() as u64).saturating_mul(vertical.ceil() as u64)
}

/// resvg's layer rectangle: group bounds widened by 2 pixels for anti-aliasing (filters
/// define their own region), clamped to `max_bbox`.
fn layer(bounds: NonZeroRect, filtered: bool, max_bbox: IntRect) -> Option<IntRect> {
    let rect = if filtered {
        IntRect::from_xywh(
            bounds.x().floor() as i32,
            bounds.y().floor() as i32,
            bounds.width().ceil().max(1.0) as u32,
            bounds.height().ceil().max(1.0) as u32,
        )?
    } else {
        IntRect::from_xywh(
            (bounds.x().floor() as i32).checked_sub(2)?,
            (bounds.y().floor() as i32).checked_sub(2)?,
            (bounds.width().ceil() as u32).checked_add(4)?,
            (bounds.height().ceil() as u32).checked_add(4)?,
        )?
    };
    fit_to_rect(rect, max_bbox)
}

/// resvg's `max_filter_bbox`: twice the canvas size before it, three times after it.
fn max_bbox(width: u32, height: u32) -> Option<IntRect> {
    IntRect::from_xywh(
        i32::try_from(width).unwrap_or(i32::MAX).saturating_mul(-2),
        i32::try_from(height).unwrap_or(i32::MAX).saturating_mul(-2),
        width.saturating_mul(5),
        height.saturating_mul(5),
    )
    .or_else(|| IntRect::from_ltrb(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2))
}

fn fit_to_rect(rect: IntRect, bounds: IntRect) -> Option<IntRect> {
    IntRect::from_ltrb(
        rect.left().max(bounds.left()),
        rect.top().max(bounds.top()),
        rect.right().min(bounds.right()),
        rect.bottom().min(bounds.bottom()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(body: &str) -> Usage {
        let mut text = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{body}</svg>"#
        );
        let document = super::super::document::parse(&mut text).unwrap();
        let tree = usvg::Tree::from_xmltree(&document, &usvg::Options::default()).unwrap();
        check(&tree, Transform::identity(), (100, 100), 0).unwrap()
    }

    const CANVAS: u64 = 100 * 100 * 4;
    const RECT: &str = r#"<rect width="100" height="100"/>"#;

    #[test]
    fn layers_clip_paths_masks_and_patterns_hold_their_buffers() {
        assert_eq!(usage(RECT).peak, CANVAS);
        // A translucent group draws into a layer at least as large as its content.
        let layer = usage(&format!(r#"<g opacity="0.5">{RECT}</g>"#)).peak;
        assert!(layer >= 2 * CANVAS, "{layer}");
        // A clip path and a mask each add a buffer of the layer's size.
        let clipped = usage(&format!(
            r##"<clipPath id="c"><rect width="50" height="50"/></clipPath><g clip-path="url(#c)">{RECT}</g>"##
        ))
        .peak;
        assert!(clipped >= layer + CANVAS, "{clipped}");
        let masked = usage(&format!(
            r##"<mask id="m"><rect width="50" height="50" fill="white"/></mask><g mask="url(#m)">{RECT}</g>"##
        ))
        .peak;
        assert!(masked >= layer + CANVAS, "{masked}");
        // A pattern tile has the pattern rectangle's device size, whatever it fills.
        let patterned = usage(
            r##"<pattern id="p" width="300" height="300" patternUnits="userSpaceOnUse"><rect width="300" height="300"/></pattern><rect width="10" height="10" fill="url(#p)"/>"##,
        )
        .peak;
        assert!(patterned >= CANVAS + 300 * 300 * 4, "{patterned}");
    }

    #[test]
    fn dashed_strokes_weigh_their_dash_count() {
        let stroke = |dashes: &str| {
            usage(&format!(
                r#"<path d="M0 50h100" stroke="black" stroke-dasharray="{dashes}"/>"#
            ))
            .work
        };
        // 50,000 dashes against 5, on the same pixels.
        assert!(stroke("0.001 0.001") > stroke("10 10") + 1_000_000);
    }

    #[test]
    fn filter_work_grows_with_kernels_and_octaves_not_with_blur_radius() {
        let work = |primitive: &str| {
            usage(&format!(
                r##"<filter id="f">{primitive}</filter><rect width="100" height="100" filter="url(#f)"/>"##
            ))
            .work
        };
        let blur =
            |deviation: u32| work(&format!(r#"<feGaussianBlur stdDeviation="{deviation}"/>"#));
        assert_eq!(blur(3), blur(50));
        // Below two pixels resvg uses its slower recursive filter.
        assert!(blur(1) > blur(3));
        let narrow = work(r#"<feMorphology radius="1"/>"#);
        let wide = work(r#"<feMorphology radius="20"/>"#);
        assert!(wide > 50 * narrow, "{narrow} {wide}");
        let one = work(r#"<feTurbulence baseFrequency="0.1" numOctaves="1"/>"#);
        let many = work(r#"<feTurbulence baseFrequency="0.1" numOctaves="64"/>"#);
        assert!(many > 30 * one, "{one} {many}");
        let small = work(r#"<feConvolveMatrix order="3" kernelMatrix="0 0 0 0 1 0 0 0 0"/>"#);
        let kernel = format!(
            r#"<feConvolveMatrix order="15" kernelMatrix="{}"/>"#,
            ["1"; 225].join(" ")
        );
        assert!(work(&kernel) > 10 * small);
    }
}
