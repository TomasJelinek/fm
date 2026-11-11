//! Read-only installed HTML startup and wheel latency probe, with bounded sampled pixel identity.
use midnight_plugin_host::CapturedFile;
use midnight_web_viewer_host::{Session, api, discover_with_diagnostics};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn signature(frame: &api::Frame) -> u64 {
    frame.rgba.iter().step_by(1009).fold(0u64, |hash, byte| {
        hash.wrapping_mul(16777619) ^ u64::from(*byte)
    })
}

fn drain(session: &Session, changed: &mut impl FnMut(api::Frame)) -> Result<(), String> {
    for event in session.drain(64) {
        match event.kind {
            api::EventKind::Frame(frame) => changed(frame),
            api::EventKind::Error { message, .. } => return Err(message),
            _ => {}
        }
    }
    if session.finished() {
        return Err("Browser stopped before probe completed".into());
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err(
            "usage: wheel_latency <installed me.exe> <fixture.html> <width> <height> <scale>"
                .into(),
        );
    }
    let app = PathBuf::from(&args[0])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let width: u32 = args[2].to_string_lossy().parse().map_err(|_| "width")?;
    let height: u32 = args[3].to_string_lossy().parse().map_err(|_| "height")?;
    let scale: f64 = args[4].to_string_lossy().parse().map_err(|_| "scale")?;
    let all = Instant::now();
    let found = discover_with_diagnostics(&app).map_err(|e| e.to_string())?;
    if !found.diagnostics.is_empty() {
        return Err(format!("{:?}", found.diagnostics));
    }
    let provider = found
        .providers
        .into_iter()
        .find(|p| p.extensions.iter().any(|e| e == "html"))
        .ok_or("HTML provider missing")?;
    println!("discovery_ms={:.3}", all.elapsed().as_secs_f64() * 1000.0);
    let source = CapturedFile::capture(&PathBuf::from(&args[1])).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let session = Session::open(
        provider,
        source,
        None,
        api::Viewport {
            revision: 1,
            width,
            height,
            scale,
        },
        Arc::new(|| true),
    )?;
    let mut previous = 0;
    let mut first = None;
    let mut last = Instant::now();
    loop {
        drain(&session, &mut |frame| {
            // The owned fixture starts with an opaque #123456 section.
            if first.is_none() && frame.rgba.get(..4) == Some(&[0x12, 0x34, 0x56, 255]) {
                first = Some(start.elapsed());
            }
            previous = signature(&frame);
            last = Instant::now();
        })?;
        if first.is_some() && last.elapsed() >= Duration::from_millis(300) {
            break;
        }
        if start.elapsed() > Duration::from_secs(30) {
            return Err("first colored frame timeout".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    println!(
        "first_content_ms={:.3} physical={}x{} scale={scale}",
        first.unwrap().as_secs_f64() * 1000.0,
        width,
        height
    );
    for round in 0..12 {
        let started = Instant::now();
        let mut first_changed = None;
        let mut frames = 0;
        let mut last_changed = started;
        if !session.send(api::CommandKind::Wheel(api::Wheel {
            x: f64::from(width / 2),
            y: f64::from(height / 2),
            delta_x: 0.0,
            delta_y: -240.0 * scale,
            modifiers: Default::default(),
        })) {
            return Err("wheel queue refused input".into());
        }
        loop {
            drain(&session, &mut |frame| {
                let next = signature(&frame);
                if next != previous {
                    previous = next;
                    first_changed.get_or_insert_with(|| started.elapsed());
                    last_changed = Instant::now();
                    frames += 1;
                }
            })?;
            if first_changed.is_some() && last_changed.elapsed() >= Duration::from_millis(200) {
                break;
            }
            if started.elapsed() > Duration::from_secs(5) {
                return Err(format!("wheel {round} timeout"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        println!(
            "wheel round={round} first_ms={:.3} last_ms={:.3} changed_frames={frames}",
            first_changed.unwrap().as_secs_f64() * 1000.0,
            last_changed.duration_since(started).as_secs_f64() * 1000.0
        );
    }
    session.close();
    Ok(())
}
