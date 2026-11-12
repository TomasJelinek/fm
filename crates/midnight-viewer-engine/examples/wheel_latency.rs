//! Separate CPU raster completion from native-window submission during wheel motion.
use midnight_core::*;
use midnight_viewer_engine::{Service, WindowSession};
use midnight_viewer_render::{TextOptions, TextRenderer, Viewport};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

fn distribution(name: &str, values: &mut [f64]) {
    values.sort_by(f64::total_cmp);
    println!(
        "{name} frames={} p50_ms={:.3} p95_ms={:.3} max_ms={:.3}",
        values.len(),
        values[values.len() / 2],
        values[values.len() * 95 / 100],
        values[values.len() - 1]
    );
}

fn raster(width: u32, height: u32, scale: f32) -> Result<(), String> {
    let mut renderer = TextRenderer::default();
    let rows = (height as f32 / (15.0 * scale) + 12.0) as usize;
    let text: String = (0..rows)
        .map(|n| {
            format!(
                "PERF_TEXT{n:08} alpha beta gamma delta 0123456789 café 世界 native scrolling\n"
            )
        })
        .collect();
    let page = renderer
        .layout_with_options(
            &text,
            &TextOptions {
                size: 15.0,
                scale,
                ..TextOptions::default()
            },
            &[],
        )
        .map_err(|e| format!("layout: {e:?}"))?;
    let mut times = Vec::new();
    for frame in 0..100 {
        let began = Instant::now();
        let output = renderer
            .rasterize(
                &page,
                Viewport {
                    width,
                    height,
                    origin: [0.0, frame as f32 * 0.76321],
                    background: [25, 29, 37, 255],
                    selection_color: [55, 86, 124, 255],
                },
                None,
            )
            .map_err(|e| format!("raster: {e:?}"))?;
        std::hint::black_box(output);
        times.push(began.elapsed().as_secs_f64() * 1000.0);
    }
    distribution(
        &format!("raster width={width} height={height} scale={scale}"),
        &mut times,
    );
    Ok(())
}

fn next(
    session: &WindowSession,
    wake: &mpsc::Receiver<()>,
    previous: Option<&ViewerFrame>,
) -> Result<ViewerFrame, String> {
    let began = Instant::now();
    loop {
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame)
                    if previous.is_none_or(|old| {
                        frame.revision > old.revision && frame.pixels != old.pixels
                    }) =>
                {
                    return Ok(*frame);
                }
                ViewerOutput::ImageUnavailable { message, .. } => return Err(message),
                _ => {}
            }
        }
        if began.elapsed() > Duration::from_secs(15) {
            return Err("completed pixel frame timed out".into());
        }
        let _ = wake.recv_timeout(Duration::from_millis(100));
    }
}

fn worker(
    service: &Service,
    path: PathBuf,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<(), String> {
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let session = service.open(
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::default(),
            focused: path.clone(),
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            archive: None,
            basic: false,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 15.0,
            foreground: [225, 229, 236, 255],
            background: [25, 29, 37, 255],
            selection: [55, 86, 124, 255],
            accent: [133, 185, 244, 255],
        },
        ViewerGeometry {
            width,
            height,
            scale,
        },
    )?;
    let (send, wake) = mpsc::sync_channel(1);
    session
        .mailbox
        .notify_on_ready(Arc::new(move || send.try_send(()).is_ok()));
    let mut current = next(&session, &wake, None)?;
    let mut times = Vec::new();
    for step in 0..100 {
        let began = Instant::now();
        let delta = if step < 50 { 0.76321 } else { -0.76321 };
        if !session.send(ViewerInput::Scroll { x: 0.0, y: delta }) {
            return Err("scroll refused".into());
        }
        current = next(&session, &wake, Some(&current))?;
        times.push(began.elapsed().as_secs_f64() * 1000.0);
    }
    distribution(
        &format!(
            "worker file={} mode={} width={width} height={height} scale={scale}",
            path.display(),
            current.mode
        ),
        &mut times,
    );
    session.close();
    Ok(())
}

fn main() -> Result<(), String> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: wheel_latency fixture-directory")?,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let service = Service::new(runtime.handle().clone());
    for (width, height, scale) in [(1000, 600, 1.0), (2250, 1400, 2.25)] {
        raster(width, height, scale)?;
        for name in ["plain.txt", "source.rs"] {
            worker(&service, directory.join(name), width, height, scale)?;
        }
    }
    Ok(())
}
