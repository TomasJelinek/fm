//! Read-only, bounded-page qualification through the production native viewer service.
use midnight_core::*;
use midnight_viewer_engine::{Service, WindowSession};
use std::{
    error::Error,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const PAGE_LIMIT: usize = 256 * 1024;
const WIDTH: u32 = 1000;
const HEIGHT: u32 = 600;

fn next_frame(
    session: &WindowSession,
    after: u64,
    position: Option<(u64, u64)>,
) -> Result<ViewerFrame, String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame)
                    if frame.revision > after
                        && position.is_none_or(|(generation, sequence)| {
                            frame.generation == generation && frame.position_sequence[0] == sequence
                        }) =>
                {
                    return Ok(*frame);
                }
                ViewerOutput::Status(status) => println!("STATUS {status:?}"),
                ViewerOutput::ImageUnavailable { message, .. } => return Err(message),
                ViewerOutput::Closed => {
                    return Err("Viewer closed before the requested frame".into());
                }
                ViewerOutput::WebState { .. } => {
                    return Err("Expected a bounded text, source, or Markdown viewer".into());
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err(format!(
        "No matching frame within 15 seconds: revision>{after}, position={position:?}"
    ))
}

fn verify(frame: &ViewerFrame, marker: Option<&str>) -> Result<(), String> {
    use midnight_plugin_host::api::viewer::{MODE_DOCUMENT, MODE_SOURCE, MODE_TEXT};
    if !matches!(frame.mode, MODE_TEXT | MODE_SOURCE | MODE_DOCUMENT) {
        return Err(format!("Unexpected viewer mode {}", frame.mode));
    }
    if frame.accessible_text.is_empty() || frame.accessible_text.len() > PAGE_LIMIT {
        return Err(format!(
            "Expected a nonempty bounded text fragment, received {} bytes",
            frame.accessible_text.len()
        ));
    }
    if frame.width != WIDTH
        || frame.height != HEIGHT
        || frame.pixels.len() != WIDTH as usize * HEIGHT as usize * 4
    {
        return Err("Unexpected raster geometry or allocation".into());
    }
    if let Some(marker) = marker
        && !frame.accessible_text.contains(marker)
    {
        return Err(format!(
            "Requested source fragment is missing marker {marker:?}"
        ));
    }
    Ok(())
}

fn report(phase: &str, elapsed: Duration, frame: &ViewerFrame) {
    println!(
        "MEASURE phase={phase} elapsed_ms={:.3} mode={} text_bytes={} raster_bytes={} \
         position={:.9} extent={:.9} generation={} sequence={} status={:?}",
        elapsed.as_secs_f64() * 1000.0,
        frame.mode,
        frame.accessible_text.len(),
        frame.pixels.len(),
        frame.position,
        frame.vertical_extent,
        frame.generation,
        frame.position_sequence[0],
        frame.status
    );
}

fn dwell(
    session: &WindowSession,
    current: &mut ViewerFrame,
    marker: Option<&str>,
) -> Result<(), String> {
    let started = Instant::now();
    let mut frames = 0;
    let mut statuses = 0;
    while started.elapsed() < Duration::from_millis(250) {
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame) if frame.revision > current.revision => {
                    verify(&frame, marker)?;
                    if frame.generation != current.generation
                        || frame.position_sequence != current.position_sequence
                        || frame.mode != current.mode
                        || (frame.position - 0.9).abs() > 0.02
                    {
                        return Err("Idle metadata update changed the acknowledged viewport".into());
                    }
                    frames += 1;
                    *current = *frame;
                }
                ViewerOutput::Status(status) => {
                    statuses += 1;
                    println!("DWELL_STATUS {status:?}");
                }
                ViewerOutput::ImageUnavailable { message, .. } => return Err(message),
                ViewerOutput::Closed => return Err("Viewer closed during the bounded dwell".into()),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    println!(
        "DWELL elapsed_ms={:.3} extra_frames={frames} statuses={statuses} final_status={:?}",
        started.elapsed().as_secs_f64() * 1000.0,
        current.status
    );
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let mut arguments = std::env::args_os().skip(1);
    let path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: large_text_probe ABSOLUTE_FILE [START_MARKER SEEK_MARKER]")?,
    );
    if !path.is_absolute() {
        return Err("The probe requires an absolute native path".into());
    }
    let mut marker = || -> Result<Option<String>, Box<dyn Error>> {
        arguments
            .next()
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| "Marker is not Unicode".into())
            })
            .transpose()
    };
    let first_marker = marker()?;
    let seek_marker = marker()?;
    if arguments.next().is_some() || first_marker.is_some() != seek_marker.is_some() {
        return Err("Supply either both markers or neither".into());
    }
    let metadata = std::fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err("The probe requires a regular file".into());
    }
    println!("INPUT path={path:?} source_bytes={}", metadata.len());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let service = Service::new(runtime.handle().clone());
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let opening = Instant::now();
    let session = service.open(
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            preferences: Arc::default(),
            focused: path,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            tree: None,
            generated: None,
            archive: None,
            basic: false,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "system-ui".into(),
            font_size: 16.0,
            foreground: [31, 35, 40, 255],
            background: [255; 4],
            selection: [180, 210, 250, 255],
            accent: [9, 105, 218, 255],
        },
        ViewerGeometry {
            width: WIDTH,
            height: HEIGHT,
            scale: 1.0,
        },
    )?;
    let mut frame = next_frame(&session, 0, None)?;
    let first_elapsed = opening.elapsed();
    verify(&frame, first_marker.as_deref())?;
    report("first", first_elapsed, &frame);
    println!(
        "FIRST_FROM_MAIN_MS={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let initial_mode = frame.mode;
    let initial_text = frame.accessible_text.clone();
    for (phase, fraction, marker) in [
        ("seek90", 0.9, seek_marker.as_deref()),
        ("return0", 0.0, first_marker.as_deref()),
    ] {
        let sequence = frame.position_sequence[0]
            .checked_add(1)
            .ok_or("Sequence exhausted")?;
        let generation = frame.generation;
        let seeking = Instant::now();
        if !session.send(ViewerInput::Position {
            generation,
            sequence,
            horizontal: false,
            position: fraction,
        }) {
            return Err("Position queue refused the probe request".into());
        }
        frame = next_frame(&session, frame.revision, Some((generation, sequence)))?;
        let elapsed = seeking.elapsed();
        verify(&frame, marker)?;
        if frame.mode != initial_mode || (frame.position - fraction).abs() > 0.02 {
            return Err(format!("Wrong acknowledged mode/position for {phase}").into());
        }
        report(phase, elapsed, &frame);
        if phase == "seek90" {
            // Leave optional source indexing alive long enough for process I/O
            // sampling; an immediate return could conceal a whole-prefix scan.
            dwell(&session, &mut frame, marker)?;
        }
    }
    if frame.accessible_text != initial_text {
        return Err("Returning to the beginning changed the source fragment".into());
    }
    session.close();
    drop(session);
    drop(service);
    runtime.shutdown_timeout(Duration::from_secs(2));
    println!(
        "PASS total_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    run()
}
