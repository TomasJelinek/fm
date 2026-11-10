//! Native installed-provider qualification. Inputs are explicit test files, never user browsing.
use midnight_plugin_host::CapturedFile;
use midnight_web_viewer_host::{Session, api, discover_with_diagnostics};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn wait(session: &Session, mut accept: impl FnMut(api::Event) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        for event in session.drain(64) {
            if let api::EventKind::Error { message, .. } = &event.kind {
                return Err(message.clone());
            }
            if accept(event) {
                return Ok(());
            }
        }
        if session.finished() {
            return Err("Browser stopped before requested result".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err("Timed out awaiting native Chromium result".into())
}

fn find(session: &Session, query: &str, expected: u32) -> Result<(), String> {
    eprintln!("Find: {query}; expected matches {expected}");
    assert!(session.send(api::CommandKind::Find(api::Find {
        query: query.into(),
        forward: true,
        match_case: false,
        find_next: false
    })));
    wait(session, |event| {
        if let api::EventKind::FindResult(value) = event.kind {
            eprintln!("Find result: {value:?}");
            value.complete && value.matches == expected
        } else {
            false
        }
    })
}

fn save_bitmap(path: &std::path::Path, frame: &api::Frame) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let bytes = frame.width * frame.height * 4;
    file.write_all(b"BM")?;
    file.write_all(&(54 + bytes).to_le_bytes())?;
    file.write_all(&[0; 4])?;
    file.write_all(&54u32.to_le_bytes())?;
    file.write_all(&40u32.to_le_bytes())?;
    file.write_all(&(frame.width as i32).to_le_bytes())?;
    file.write_all(&(-(frame.height as i32)).to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&32u16.to_le_bytes())?;
    file.write_all(&[0; 24])?;
    for pixel in frame.rgba.as_chunks::<4>().0 {
        file.write_all(&[pixel[2], pixel[1], pixel[0], pixel[3]])?;
    }
    file.flush()?;
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.len() != 3 {
        return Err("Usage: probe <installed me.exe> <fixture.html> <report directory>".into());
    }
    let discovered = discover_with_diagnostics(&args[0]).map_err(|e| e.to_string())?;
    if !discovered.diagnostics.is_empty() {
        return Err(format!(
            "Discovery diagnostics: {:?}",
            discovered.diagnostics
        ));
    }
    let provider = discovered
        .providers
        .into_iter()
        .find(|p| p.extensions.iter().any(|e| e == "html"))
        .ok_or("Installed HTML provider not found")?;
    let source = CapturedFile::capture(&args[1]).map_err(|e| e.to_string())?;
    let session = Session::open(
        provider,
        source,
        None,
        api::Viewport {
            revision: 1,
            width: 640,
            height: 480,
            scale: 1.0,
        },
        Arc::new(|| true),
    )?;
    let started = Instant::now();
    std::fs::create_dir_all(&args[2]).map_err(|e| e.to_string())?;
    let mut first_ms = 0;
    wait(&session, |event| {
        if let api::EventKind::Frame(frame) = event.kind {
            let colored = frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] < 80 && p[1] > 120 && p[2] < 100)
                .count();
            if colored > 1000 {
                first_ms = started.elapsed().as_millis();
                save_bitmap(&args[2].join("html-offline.bmp"), &frame).unwrap();
                return true;
            }
        }
        false
    })?;
    find(&session, "Chromium needle", 2)?;
    find(&session, "Script executed", 0)?;
    eprintln!("Resize: 801x601 at 125% DPI");
    assert!(session.send(api::CommandKind::Resize(api::Viewport {
        revision: 2,
        width: 801,
        height: 601,
        scale: 1.25
    })));
    wait(&session, |event| {
        matches!(event.kind,api::EventKind::Frame(frame)
        if frame.viewport_revision==2&&frame.width==801&&frame.height==601)
    })?;
    assert!(session.send(api::CommandKind::Zoom(1.5)));
    eprintln!("Enable online JavaScript in a fresh document");
    assert!(session.send(api::CommandKind::Policy(api::Policy {
        network: true,
        scripts: true
    })));
    wait(&session, |event| {
        matches!(event.kind, api::EventKind::Frame(_))
    })?;
    find(&session, "Script executed", 1)?;
    eprintln!("Return offline in a fresh document");
    assert!(session.send(api::CommandKind::Policy(api::Policy::default())));
    wait(&session, |event| {
        matches!(event.kind, api::EventKind::Frame(_))
    })?;
    find(&session, "Script executed", 0)?;
    let closed = Instant::now();
    session.close();
    drop(session);
    println!(
        "{{\"first_content_ms\":{first_ms},\"close_ms\":{},\"find\":true,\"offline_script_denial\":true,\"policy_recreation\":true,\"fractional_dpi\":true}}",
        closed.elapsed().as_millis()
    );
    Ok(())
}
