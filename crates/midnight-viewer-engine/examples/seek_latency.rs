//! Bounded production scrollbar pipeline timing without a native window or pixel upload.
use midnight_core::*;
use midnight_viewer_engine::{Service, WindowSession};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn first(session: &WindowSession) -> Result<ViewerFrame, String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        for output in session.mailbox.take() {
            if let ViewerOutput::Frame(frame) = output {
                if frame.raster.is_some() {
                    return Ok(*frame);
                }
            } else if let ViewerOutput::Status(status) = output {
                eprintln!("status: {status}");
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err("initial retained frame timed out".into())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: seek_latency FILE")?,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let service = Service::new(runtime.handle().clone());
    let scale = std::env::var("MIDNIGHT_VIEWER_SEEK_SCALE")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| (0.5..=3.0).contains(value))
        .unwrap_or(2.25);
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let session = service.open(
        ViewerLaunch {
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::default(),
            focused: path,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            archive: None,
            basic: false,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "Segoe UI".into(),
            font_size: 13.0,
            foreground: [225, 229, 236, 255],
            background: [25, 29, 37, 255],
            selection: [55, 86, 124, 255],
            accent: [133, 185, 244, 255],
        },
        ViewerGeometry {
            width: (1312.0 * scale).round() as u32,
            height: (689.0 * scale).round() as u32,
            scale,
        },
    )?;
    if !session.send(ViewerInput::RetainedRaster(true)) {
        return Err("input refused".into());
    }
    let frame = first(&session)?;
    let generation = frame.generation;
    println!(
        "initial mode={} viewport={:?} bytes={} raster={}x{}",
        frame.mode,
        frame.raster.as_ref().map(|info| info.viewport),
        frame.accessible_text.len(),
        frame.width,
        frame.height
    );
    service.retire(frame);
    let mut sent = Vec::new();
    let mut rows = Vec::new();
    let mut final_received = false;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if sent.len() < 60
            && sent
                .last()
                .is_none_or(|last: &Instant| last.elapsed() >= Duration::from_millis(10))
        {
            let segment = sent.len() / 20;
            let fraction = (sent.len() % 20 + 1) as f64 / 20.0;
            let (from, to) = [(0.1, 0.8), (0.8, 0.2), (0.2, 0.7)][segment];
            if !session.send(ViewerInput::Position {
                generation,
                sequence: sent.len() as u64 + 1,
                horizontal: false,
                position: from + (to - from) * fraction,
            }) {
                return Err("position refused".into());
            }
            sent.push(Instant::now());
        }
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame) => {
                    let sequence = frame.position_sequence[0];
                    if sequence != 0 {
                        let elapsed = sent[sequence as usize - 1].elapsed().as_secs_f64() * 1000.0;
                        rows.push(format!(
                            "frame seq={sequence} latency_ms={elapsed:.3} at_ms={:.3} \
                             position={:.9} bytes={} raster={}x{}",
                            started.elapsed().as_secs_f64() * 1000.0,
                            frame.position,
                            frame.accessible_text.len(),
                            frame.width,
                            frame.height,
                        ));
                        final_received |= sequence == 60;
                    }
                    service.retire(frame);
                }
                ViewerOutput::Status(status) => eprintln!("status: {status}"),
                _ => {}
            }
        }
        if final_received {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    for row in &rows {
        println!("{row}");
    }
    println!(
        "requests={} frames={} final_received={final_received}",
        sent.len(),
        rows.len()
    );
    session.close();
    if !final_received {
        return Err("final position timed out".into());
    }
    Ok(())
}
