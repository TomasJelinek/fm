//! Installed HTML provider idle and frame-rate probe. It opens one owned fixture, waits for
//! the first frame (of a scripts-enabled online document with `scripts`), then holds the
//! session open for the given seconds while it counts the frames that arrive, and closes.
//! The process itself never polls: its main thread waits for the session's wakes, so an
//! external sampler can attribute every context switch of an idle session to the host.
//! Lines on standard output: `ready ...`, `idle_start`, `idle_end ...`, `closed ...`.
use midnight_plugin_host::CapturedFile;
use midnight_web_viewer_host::{Session, api, discover_with_diagnostics};
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

/// Session wakes, counted by the actor thread; the main thread waits for the next one.
#[derive(Default)]
struct Wakes {
    count: Mutex<u64>,
    changed: Condvar,
}

impl Wakes {
    fn wake(&self) -> bool {
        *self.count.lock().unwrap_or_else(|error| error.into_inner()) += 1;
        self.changed.notify_all();
        true
    }
    /// Wait until a wake after `seen`, or until `deadline`; return the wake count.
    fn after(&self, seen: u64, deadline: Instant) -> u64 {
        let count = self.count.lock().unwrap_or_else(|error| error.into_inner());
        let left = deadline.saturating_duration_since(Instant::now());
        let (count, _) = self
            .changed
            .wait_timeout_while(count, left, |count| *count == seen)
            .unwrap_or_else(|error| error.into_inner());
        *count
    }
}

/// Drain the session; count the frames of the current document. Errors end the probe.
fn drain(session: &Session, document_frames: &mut u64) -> Result<(), String> {
    for event in session.drain(64) {
        match event.kind {
            api::EventKind::Frame(_) => *document_frames += 1,
            api::EventKind::Error { message, .. } => return Err(message),
            api::EventKind::Closed => return Err("Browser closed".into()),
            _ => {}
        }
    }
    if session.finished() {
        return Err("Browser stopped".into());
    }
    Ok(())
}

/// Wait, without polling, until the session delivered a frame of its current document.
fn first_frame(session: &Session, wakes: &Wakes, deadline: Instant) -> Result<(), String> {
    let mut seen = 0;
    let mut frames = 0;
    loop {
        drain(session, &mut frames)?;
        if frames > 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("first frame timeout".into());
        }
        seen = wakes.after(seen, deadline);
    }
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !(6..=7).contains(&args.len()) {
        return Err(
            "usage: html_idle <installed midnight-explorer.exe> <fixture.html> <seconds> <width> \
                    <height> <scale> [offline|scripts]"
                .into(),
        );
    }
    let number = |index: usize, name: &str| {
        args[index]
            .to_string_lossy()
            .parse::<f64>()
            .map_err(|_| format!("invalid {name}"))
    };
    let seconds = number(2, "seconds")?;
    let width = number(3, "width")? as u32;
    let height = number(4, "height")? as u32;
    let scale = number(5, "scale")?;
    let scripts = match args.get(6).map(|value| value.to_string_lossy()) {
        None => false,
        Some(mode) if mode == "offline" => false,
        Some(mode) if mode == "scripts" => true,
        Some(_) => return Err("mode must be offline or scripts".into()),
    };
    let app = PathBuf::from(&args[0])
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let found = discover_with_diagnostics(&app).map_err(|error| error.to_string())?;
    if !found.diagnostics.is_empty() {
        return Err(format!("{:?}", found.diagnostics));
    }
    let provider = found
        .providers
        .into_iter()
        .find(|provider| provider.extensions.iter().any(|value| value == "html"))
        .ok_or("HTML provider missing")?;
    let source =
        CapturedFile::capture(&PathBuf::from(&args[1])).map_err(|error| error.to_string())?;
    let wakes = Arc::new(Wakes::default());
    let wake = {
        let wakes = wakes.clone();
        Arc::new(move || wakes.wake())
    };
    let started = Instant::now();
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
        wake,
    )?;
    let deadline = Instant::now() + Duration::from_secs(30);
    first_frame(&session, &wakes, deadline)?;
    if scripts {
        if !session.send(api::CommandKind::Policy(api::Policy {
            network: true,
            scripts: true,
        })) {
            return Err("policy refused".into());
        }
        first_frame(&session, &wakes, deadline)?;
    }
    let mut out = std::io::stdout().lock();
    let ready = started.elapsed().as_secs_f64() * 1000.0;
    let pid = std::process::id();
    writeln!(out, "ready pid={pid} first_frame_ms={ready:.3}").map_err(|e| e.to_string())?;
    writeln!(out, "idle_start").map_err(|error| error.to_string())?;
    out.flush().map_err(|error| error.to_string())?;
    let idle = Instant::now();
    let end = idle + Duration::from_secs_f64(seconds);
    let mut frames = 0;
    let mut seen = *wakes
        .count
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    while Instant::now() < end {
        drain(&session, &mut frames)?;
        seen = wakes.after(seen, end);
    }
    drain(&session, &mut frames)?;
    let held = idle.elapsed().as_secs_f64();
    writeln!(
        out,
        "idle_end frames={frames} seconds={held:.3} fps={:.2}",
        frames as f64 / held
    )
    .map_err(|error| error.to_string())?;
    out.flush().map_err(|error| error.to_string())?;
    let closing = Instant::now();
    session.close();
    drop(session);
    writeln!(
        out,
        "closed close_ms={:.3}",
        closing.elapsed().as_secs_f64() * 1000.0
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
