//! Synthetic text-heavy Letter PDFs and in-process page render timings.
//!
//! `render_probe [--write DIR] [--quick]` generates two documents like a typical report:
//! headings, justified body text, bullet lists, a table with 0.5 pt borders and a small
//! raster logo on every page (3 and 300 pages). `--write` also stores them in DIR for the
//! host's end-to-end probe. Timings call the plugin's own `Document::render`, without the
//! sandbox, the pipe or the host; they separate rasterization cost from transport cost.
use midnight_image_viewer_sdk::{Document, api::Viewport};
use std::{io::Cursor, path::PathBuf, time::Instant};

const WORDS: &[&str] = &[
    "report",
    "quarterly",
    "synthetic",
    "document",
    "revenue",
    "analysis",
    "summary",
    "the",
    "of",
    "and",
    "to",
    "in",
    "for",
    "with",
    "on",
    "by",
    "operations",
    "review",
    "customer",
    "service",
    "growth",
    "measure",
    "result",
    "target",
    "variance",
    "plan",
    "schedule",
    "delivery",
    "quality",
    "process",
    "system",
    "average",
    "region",
    "segment",
    "forecast",
    "budget",
    "actual",
    "percent",
    "increase",
    "decrease",
];

struct Words(u64);
impl Words {
    fn next(&mut self) -> &'static str {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        WORDS[(self.0 >> 33) as usize % WORDS.len()]
    }
    fn line(&mut self, count: usize) -> String {
        (0..count)
            .map(|_| self.next())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// One Letter page: a heading, body paragraphs, two bullet lists, a bordered table and
/// a 64x64 RGB logo drawn at 48 pt.
fn page_content(page: usize, words: &mut Words) -> String {
    let mut content = String::new();
    content.push_str("q 48 0 0 48 492 718 cm /Logo Do Q\n");
    content.push_str(&format!(
        "BT /F2 18 Tf 72 730 Td (Section {} - {}) Tj ET\n",
        page + 1,
        words.line(4)
    ));
    let mut y = 700.0;
    for paragraph in 0..3 {
        for _ in 0..7 {
            content.push_str(&format!(
                "BT /F1 10.5 Tf 72 {y:.1} Td ({}) Tj ET\n",
                words.line(14)
            ));
            y -= 13.0;
        }
        y -= 6.0;
        for _ in 0..4 {
            content.push_str(&format!(
                "BT /F1 10.5 Tf 84 {y:.1} Td (\\225) Tj 12 0 Td ({}) Tj ET\n",
                words.line(9 + paragraph)
            ));
            y -= 13.0;
        }
        y -= 8.0;
    }
    let (left, right, top, rows, columns) = (72.0, 540.0, y - 4.0, 8, 4);
    let height = 16.0;
    content.push_str("0.5 w 0 0 0 RG\n");
    for row in 0..=rows {
        let line = top - f64::from(row) * height;
        content.push_str(&format!("{left} {line:.1} m {right} {line:.1} l S\n"));
    }
    let bottom = top - f64::from(rows) * height;
    for column in 0..=columns {
        let x = left + (right - left) * f64::from(column) / f64::from(columns);
        content.push_str(&format!("{x:.1} {top:.1} m {x:.1} {bottom:.1} l S\n"));
    }
    for row in 0..rows {
        for column in 0..columns {
            let x = left + 4.0 + (right - left) * f64::from(column) / f64::from(columns);
            let base = top - f64::from(row) * height - 11.5;
            let font = if row == 0 { "F2" } else { "F3" };
            content.push_str(&format!(
                "BT /{font} 9 Tf {x:.1} {base:.1} Td ({}) Tj ET\n",
                words.line(2)
            ));
        }
    }
    content.push_str(&format!(
        "BT /F3 8 Tf 300 40 Td (Page {} of the synthetic report) Tj ET\n",
        page + 1
    ));
    content
}

/// A valid cross-reference PDF with `pages` Letter pages sharing fonts and the logo.
pub fn report(pages: usize) -> Vec<u8> {
    let mut words = Words(pages as u64 * 7919);
    let first_page = 7;
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            (0..pages)
                .map(|page| format!("{} 0 R", first_page + 2 * page))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    let mut logo = Vec::with_capacity(64 * 64 * 3);
    for y in 0..64u32 {
        for x in 0..64u32 {
            let inside = (x as i32 - 32).pow(2) + (y as i32 - 32).pow(2) < 28 * 28;
            logo.extend_from_slice(&if inside {
                [(x * 4) as u8, 40, (255 - y * 4) as u8]
            } else {
                [255, 255, 255]
            });
        }
    }
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width 64 /Height 64 /ColorSpace /DeviceRGB \
         /BitsPerComponent 8 /Length {} >>\nstream\n",
        logo.len()
    )
    .into_bytes();
    image.extend_from_slice(&logo);
    image.extend_from_slice(b"\nendstream");
    objects.push(image);
    for page in 0..pages {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << \
                 /F1 3 0 R /F2 4 0 R /F3 5 0 R >> /XObject << /Logo 6 0 R >> >> /Contents {} 0 R >>",
                first_page + 2 * page + 1
            )
            .into_bytes(),
        );
        let content = page_content(page, &mut words);
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .into_bytes(),
        );
    }
    let mut bytes = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

const PAGE: [f64; 2] = [612.0, 792.0];
const GAP: f64 = 24.0;

/// The host's former per-step work: every visible page slice of one bounded viewport.
fn viewport_step(document: &impl Document, viewport: [u32; 2], zoom: f64, top: f64) -> usize {
    let [width, height] = viewport.map(f64::from);
    let density = (4096.0 / width)
        .min(4096.0 / height)
        .min((4_194_304.0 / (width * height)).sqrt())
        .min(1.0);
    let size = [(width * density) as u32, (height * density) as u32];
    let scale = zoom * density;
    let left = (PAGE[0] - width / zoom) / 2.0;
    let mut pixels = 0;
    for page in 0..3usize {
        let origin = page as f64 * (PAGE[1] + GAP);
        let low_y = ((origin - top) * scale)
            .floor()
            .clamp(0.0, f64::from(size[1]));
        let high_y = ((origin + PAGE[1] - top) * scale)
            .ceil()
            .clamp(0.0, f64::from(size[1]));
        let low_x = ((0.0 - left) * scale)
            .floor()
            .clamp(0.0, f64::from(size[0]));
        let high_x = ((PAGE[0] - left) * scale)
            .ceil()
            .clamp(0.0, f64::from(size[0]));
        if high_y <= low_y || high_x <= low_x {
            continue;
        }
        let slice = Viewport {
            width: (high_x - low_x) as u32,
            height: (high_y - low_y) as u32,
            scale,
            origin_x: left + low_x / scale,
            origin_y: top + low_y / scale - origin,
        };
        let frame = document.render(page as u32, slice).expect("slice render");
        pixels += frame.rgba.len() / 4;
    }
    pixels
}

/// Hayro directly: the same page band rendered with a fresh or a shared `RenderCache`.
fn cache_experiment(bytes: &[u8]) {
    use hayro::{
        RenderCache, RenderSettings,
        hayro_interpret::{InterpreterSettings, util::TransformExt},
        hayro_syntax::Pdf,
        kurbo::{Affine, Rect},
        vello_cpu::{
            Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit,
            color::palette::css::{TRANSPARENT, WHITE},
        },
    };
    let pdf = Pdf::new(bytes.to_vec()).expect("parse");
    let pages = pdf.pages();
    fn band_of<'a>(
        page: &'a hayro::hayro_syntax::page::Page<'a>,
        scale: f64,
        top: u32,
        size: [u32; 2],
        cache: &RenderCache<'a>,
    ) {
        let mut context = RenderContext::new(size[0] as u16, size[1] as u16);
        let mapping = Affine::scale(scale) * Affine::translate((0.0, -f64::from(top) / scale));
        context.set_transform(mapping);
        context.set_paint(WHITE);
        context.fill_rect(&Rect::new(0.0, 0.0, PAGE[0], PAGE[1]));
        hayro::render_into(
            page,
            cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &mut context,
            mapping * page.initial_transform(true).to_kurbo(),
        );
        context.flush();
        let mut pixmap = Pixmap::new(size[0] as u16, size[1] as u16);
        context.render_with(
            &mut pixmap,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(TRANSPARENT),
                ..Default::default()
            },
        );
        std::hint::black_box(pixmap);
    }
    for zoom in [1.641, 3.283, 4.0] {
        let width = (PAGE[0] * zoom).ceil() as u32;
        let height = (PAGE[1] * zoom).ceil() as u32;
        let rows = 1024u32;
        let mut fresh = 0.0;
        let mut shared = 0.0;
        let mut second = 0.0;
        for page in 0..3 {
            let began = Instant::now();
            let mut top = 0;
            while top < height {
                band_of(
                    &pages[page],
                    zoom,
                    top,
                    [width, rows.min(height - top)],
                    &RenderCache::new(),
                );
                top += rows;
            }
            fresh += began.elapsed().as_secs_f64() * 1000.0;
            let cache = RenderCache::new();
            let began = Instant::now();
            let mut top = 0;
            while top < height {
                band_of(
                    &pages[page],
                    zoom,
                    top,
                    [width, rows.min(height - top)],
                    &cache,
                );
                top += rows;
            }
            shared += began.elapsed().as_secs_f64() * 1000.0;
            let began = Instant::now();
            let mut top = 0;
            while top < height {
                band_of(
                    &pages[page],
                    zoom,
                    top,
                    [width, rows.min(height - top)],
                    &cache,
                );
                top += rows;
            }
            second += began.elapsed().as_secs_f64() * 1000.0;
        }
        let low = Instant::now();
        for page in 0..3 {
            let scale = zoom / 4.0;
            band_of(
                &pages[page],
                scale,
                0,
                [
                    (PAGE[0] * scale).ceil() as u32,
                    (PAGE[1] * scale).ceil() as u32,
                ],
                &RenderCache::new(),
            );
        }
        println!(
            "cache zoom={:.1}% page={width}x{height} bands_of={rows} fresh_ms_per_page={:.1} \
             shared_ms_per_page={:.1} rerender_shared_ms_per_page={:.1} quarter_scale_ms_per_page={:.1}",
            zoom * 100.0,
            fresh / 3.0,
            shared / 3.0,
            second / 3.0,
            low.elapsed().as_secs_f64() * 1000.0 / 3.0
        );
    }
}

fn main() {
    let mut write = None;
    let mut quick = false;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--write" => write = arguments.next().map(PathBuf::from),
            "--quick" => quick = true,
            other => panic!("unknown argument {other}"),
        }
    }
    let short = report(3);
    let long = report(300);
    if let Some(directory) = &write {
        std::fs::create_dir_all(directory).expect("fixture directory");
        std::fs::write(directory.join("report-3.pdf"), &short).expect("write 3-page fixture");
        std::fs::write(directory.join("report-300.pdf"), &long).expect("write 300-page fixture");
        println!(
            "wrote {} ({} bytes) and report-300.pdf ({} bytes)",
            directory.join("report-3.pdf").display(),
            short.len(),
            long.len()
        );
    }
    cache_experiment(&short);
    let began = Instant::now();
    let document = midnight_viewer_pdf::decode(&mut Cursor::new(long)).expect("decode");
    println!(
        "decode pages=300 ms={:.1}",
        began.elapsed().as_secs_f64() * 1000.0
    );
    let document_3 = midnight_viewer_pdf::decode(&mut Cursor::new(short)).expect("decode");
    let steps = if quick { 6 } else { 24 };
    for device in [1.0, 2.0] {
        let viewport = [(2000.0 * device) as u32, (1300.0 * device) as u32];
        let fit = f64::from(viewport[1]) / PAGE[1];
        for (name, zoom) in [("fit", fit), ("100", 1.0), ("200", 2.0), ("400", 4.0)] {
            // Former pipeline: each scroll step re-rendered every visible slice.
            let mut times = Vec::new();
            let mut pixels = 0;
            for step in 0..steps {
                let top = step as f64 * 90.0 * device / zoom;
                let began = Instant::now();
                pixels = viewport_step(&document_3, viewport, zoom, top);
                times.push(began.elapsed().as_secs_f64() * 1000.0);
            }
            let median = percentile(&mut times, 0.5);
            let p95 = percentile(&mut times, 0.95);
            // Whole-page cost at the exact physical scale, in bands of at most 4 Mpx.
            let page_size = PAGE.map(|edge| (edge * zoom).ceil() as u32);
            let rows = (4_194_304 / page_size[0].min(4096)).clamp(1, 4096);
            let mut page_times = Vec::new();
            for page in [0u32, 1, 2] {
                let began = Instant::now();
                let mut y = 0;
                while y < page_size[1] {
                    let height = rows.min(page_size[1] - y);
                    let mut x = 0;
                    while x < page_size[0] {
                        let width = 4096.min(page_size[0] - x);
                        let frame = document
                            .render(
                                page,
                                Viewport {
                                    width,
                                    height,
                                    scale: zoom,
                                    origin_x: f64::from(x) / zoom,
                                    origin_y: f64::from(y) / zoom,
                                },
                            )
                            .expect("band render");
                        std::hint::black_box(frame);
                        x += width;
                    }
                    y += height;
                }
                page_times.push(began.elapsed().as_secs_f64() * 1000.0);
            }
            println!(
                "device={device} zoom={name} ({:.1}%) viewport={}x{} step_pixels={pixels} \
                 step_p50_ms={median:.1} step_p95_ms={p95:.1} page={}x{} page_ms={:.1}/{:.1}/{:.1}",
                zoom * 100.0,
                viewport[0],
                viewport[1],
                page_size[0],
                page_size[1],
                page_times[0],
                page_times[1],
                page_times[2],
            );
        }
    }
}
