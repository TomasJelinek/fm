//! Opt-in rasterizer timings for before/after comparisons. Run optimized:
//! `cargo test --release -p midnight-viewer-render --test integration raster_timing -- --ignored --nocapture`
use midnight_viewer_render::{ColorSpan, TextOptions, TextPage, TextRenderer, Viewport};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// A retained viewer raster: a 1920x1080 viewport with 256-pixel guards.
fn retained(origin: [f32; 2]) -> Viewport {
    Viewport {
        width: 1920,
        height: 1080 + 2 * 256,
        origin,
        background: [24, 27, 34, 255],
        selection_color: [45, 80, 140, 255],
    }
}

fn prose(bytes: usize) -> String {
    let words = [
        "viewer",
        "retained",
        "raster",
        "glyph",
        "phase",
        "Unicode",
        "Příliš",
        "žluťoučký",
        "kůň",
        "paragraph",
        "proportional",
        "line",
        "wraps",
        "naturally",
        "and",
        "keeps",
        "fractional",
        "advances",
        "of",
        "the",
        "font",
    ];
    let mut text = String::new();
    let mut index = 0usize;
    while text.len() < bytes {
        text.push_str(words[index % words.len()]);
        index = index.wrapping_mul(7).wrapping_add(3) % 997;
        text.push(if index.is_multiple_of(23) { '\n' } else { ' ' });
    }
    text.truncate(text.floor_char_boundary(bytes));
    text
}

fn colored_source(bytes: usize) -> (String, Vec<ColorSpan>) {
    let line = "    let value = compute(index, \"text\") + 42; // comment\n";
    let mut text = String::new();
    while text.len() + line.len() <= bytes {
        text.push_str(line);
    }
    let palette = [
        [86, 156, 214, 255],
        [206, 145, 120, 255],
        [181, 206, 168, 255],
        [106, 153, 85, 255],
        [220, 220, 170, 255],
    ];
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, word) in text.split_inclusive(' ').enumerate() {
        if spans.len() == 4096 {
            break;
        }
        spans.push(ColorSpan {
            range: start..start + word.len(),
            color: palette[index % palette.len()],
        });
        start += word.len();
    }
    (text, spans)
}

fn time(mut operation: impl FnMut()) -> Duration {
    let started = Instant::now();
    operation();
    started.elapsed()
}

fn report(name: &str, renderer: &mut TextRenderer, page: &TextPage, origin: [f32; 2]) {
    let cold = time(|| {
        renderer.rasterize(page, retained(origin), None).unwrap();
    });
    let mut repaint = Duration::MAX;
    for _ in 0..5 {
        repaint = repaint.min(time(|| {
            renderer.rasterize(page, retained(origin), None).unwrap();
        }));
    }
    let scroll = time(|| {
        for step in 1..=10 {
            let origin = [origin[0], origin[1] + step as f32 * 37.3];
            renderer.rasterize(page, retained(origin), None).unwrap();
        }
    });
    println!(
        "raster_timing case={name} cold_ms={:.2} repaint_ms={:.2} scroll10_ms={:.2}",
        cold.as_secs_f64() * 1000.0,
        repaint.as_secs_f64() * 1000.0,
        scroll.as_secs_f64() * 1000.0,
    );
}

/// One retained refill handed over as a shared `Arc<[u8]>` frame: rasterize and
/// copy the `Vec` into an `Arc` (the engine's current path), or allocate the Arc
/// once and paint it while it is still unique. Best of alternating runs.
fn shared_delivery(name: &str, renderer: &mut TextRenderer, page: &TextPage, origin: [f32; 2]) {
    let view = retained(origin);
    let bytes = view.width as usize * view.height as usize * 4;
    let mut copied = Duration::MAX;
    let mut in_place = Duration::MAX;
    for _ in 0..9 {
        copied = copied.min(time(|| {
            let raster = renderer.rasterize(page, view, None).unwrap();
            std::hint::black_box(Arc::<[u8]>::from(raster.rgba));
        }));
        in_place = in_place.min(time(|| {
            let mut pixels: Arc<[u8]> = std::iter::repeat_n(0, bytes).collect();
            renderer
                .rasterize_into(page, view, None, Arc::get_mut(&mut pixels).unwrap())
                .unwrap();
            std::hint::black_box(pixels);
        }));
    }
    println!(
        "raster_timing case={name} bytes={bytes} vec_then_arc_copy_ms={:.2} arc_in_place_ms={:.2}",
        copied.as_secs_f64() * 1000.0,
        in_place.as_secs_f64() * 1000.0,
    );
}

/// B8-V1 decision input: a retained refill allocates a zeroed shared buffer and then
/// clears it to the background; a buffer collected already filled with the background
/// would skip one full pass. Best of alternating runs for a 22 MB refill.
#[test]
#[ignore = "timing harness; run optimized with --ignored --nocapture"]
fn background_prefill_timing() {
    let background = [24u8, 27, 34, 255];
    let bytes = 1920 * 2867 * 4;
    let mut zero_then_clear = Duration::MAX;
    let mut prefilled = Duration::MAX;
    for _ in 0..15 {
        zero_then_clear = zero_then_clear.min(time(|| {
            let mut pixels: Arc<[u8]> = std::iter::repeat_n(0, bytes).collect();
            Arc::get_mut(&mut pixels)
                .unwrap()
                .as_chunks_mut::<4>()
                .0
                .fill(background);
            std::hint::black_box(pixels);
        }));
        prefilled = prefilled.min(time(|| {
            let pixels: Arc<[u8]> = (0..bytes).map(|index| background[index & 3]).collect();
            std::hint::black_box(pixels);
        }));
    }
    println!(
        "raster_timing case=prefill bytes={bytes} zero_then_clear_ms={:.2} prefilled_ms={:.2} saved_ms={:.2}",
        zero_then_clear.as_secs_f64() * 1000.0,
        prefilled.as_secs_f64() * 1000.0,
        (zero_then_clear.as_secs_f64() - prefilled.as_secs_f64()) * 1000.0,
    );
}

#[test]
#[ignore = "timing harness; run optimized with --ignored --nocapture"]
fn raster_timing() {
    let mut renderer = TextRenderer::default();
    let text = prose(64 * 1024);
    let page = renderer
        .layout_with_options(
            &text,
            &TextOptions {
                family: "sans-serif",
                size: 16.0,
                wrap_width: Some(1800.0),
                line_height: Some(1.5),
                ..TextOptions::default()
            },
            &[],
        )
        .unwrap();
    let middle = [0.0, (page.height() / 2.0).floor() + 0.375];
    report("prose_64k", &mut renderer, &page, middle);
    shared_delivery("prose_64k_shared", &mut renderer, &page, middle);

    let (source, spans) = colored_source(64 * 1024);
    let page = renderer
        .layout_with_options(
            &source,
            &TextOptions {
                size: 14.0,
                ..TextOptions::default()
            },
            &spans,
        )
        .unwrap();
    let middle = [0.0, (page.height() / 2.0).floor() + 0.25];
    report("colored_source_64k", &mut renderer, &page, middle);

    let line = "abcdefgh".repeat(32768);
    let page = renderer.layout(&line, 18.0, 1.0).unwrap();
    report("line_256k", &mut renderer, &page, [100_000.25, 0.0]);
}
