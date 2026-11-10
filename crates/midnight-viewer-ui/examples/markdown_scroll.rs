//! Exercise the production Markdown worker on a real read-only file and compare scrolling pixels.
use midnight_core::*;
use midnight_viewer_engine::{Service, WindowSession};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn frame(session: &WindowSession, after: u64) -> Result<ViewerFrame, String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame) if frame.revision > after => return Ok(*frame),
                ViewerOutput::Status(status) => eprintln!("Status: {status}"),
                ViewerOutput::ImageUnavailable { message, .. } => return Err(message),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err(format!("Frame timed out after revision {after}"))
}

fn save(frame: &ViewerFrame, path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(file, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&frame.pixels)?;
    Ok(())
}

fn mismatches(before: &ViewerFrame, after: &ViewerFrame, delta: i32) -> f64 {
    let stride = before.width as usize * 4;
    let shift = delta.unsigned_abs() as usize * stride;
    let end = before.pixels.len() - shift;
    let (old, new) = if delta > 0 {
        (&before.pixels[shift..], &after.pixels[..end])
    } else {
        (&before.pixels[..end], &after.pixels[shift..])
    };
    let changed = old
        .as_chunks::<4>()
        .0
        .iter()
        .zip(new.as_chunks::<4>().0)
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
        .count();
    changed as f64 / (old.len() / 4) as f64
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path =
        PathBuf::from(arguments.next().ok_or(
            "usage: markdown_scroll file output-directory [step-pixels [width [height]]]",
        )?);
    let output = PathBuf::from(arguments.next().ok_or("missing output directory")?);
    let mut number = |default| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(match arguments.next() {
            Some(value) => value.to_str().ok_or("non-Unicode number")?.parse()?,
            None => default,
        })
    };
    let step = number(72)?;
    let width = number(1000)?;
    let height = number(600)?;
    if step == 0 || step >= height || width < 320 || !(200..=2000).contains(&height) {
        return Err("invalid scrolling probe geometry".into());
    }
    std::fs::create_dir_all(&output)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let service = Service::new(runtime.handle().clone());
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
            family: "system-ui".into(),
            font_size: 16.0,
            foreground: [31, 35, 40, 255],
            background: [255; 4],
            selection: [180, 210, 250, 255],
            accent: [9, 105, 218, 255],
        },
        ViewerGeometry {
            width,
            height,
            scale: 1.0,
        },
    )?;
    let mut current = frame(&session, 0)?;
    let initial = current.pixels.clone();
    save(&current, output.join("initial.png"))?;
    let mut worst = 0.0_f64;
    let mut latencies = Vec::new();
    let mut reports = Vec::new();
    for round in 0..2 {
        for delta in [step as i32, -(step as i32)] {
            let mut complete = false;
            for step in 0..2000 {
                let start = Instant::now();
                if !session.send(ViewerInput::Scroll {
                    x: 0.0,
                    y: delta as f32,
                }) {
                    return Err("scroll queue refused".into());
                }
                let next = frame(&session, current.revision)?;
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                latencies.push(elapsed);
                let edge = if delta > 0 {
                    next.position >= 0.999999
                } else {
                    next.position <= 0.000001
                };
                let mismatch = if edge {
                    0.0
                } else {
                    mismatches(&current, &next, delta)
                };
                worst = worst.max(mismatch);
                reports.push(format!(
                    "{round},{delta},{step},{:.7},{elapsed:.3},{mismatch:.7},{}",
                    next.position, next.status
                ));
                if mismatch > 0.001 {
                    save(&current, output.join("before-mismatch.png"))?;
                    save(&next, output.join("after-mismatch.png"))?;
                    std::fs::write(output.join("scroll.csv"), reports.join("\n"))?;
                    return Err(format!("Unstable scrolling: {:.3}% pixels changed at round {round}, step {step}, delta {delta}", mismatch * 100.0).into());
                }
                current = next;
                if edge {
                    save(&current, output.join(format!("round-{round}-{delta}.png")))?;
                    complete = true;
                    break;
                }
            }
            if !complete {
                return Err("Scrolling did not reach document edge".into());
            }
        }
        if current.pixels != initial {
            std::fs::write(output.join("scroll.csv"), reports.join("\n"))?;
            return Err("Top pixels differ after full down/up traversal".into());
        }
    }
    std::fs::write(output.join("scroll.csv"), reports.join("\n"))?;
    latencies.sort_by(f64::total_cmp);
    println!(
        "PASS: {} scrolls, mismatch max={worst:.7}, p50={:.2}ms p95={:.2}ms",
        latencies.len(),
        latencies[latencies.len() / 2],
        latencies[latencies.len() * 95 / 100]
    );
    session.close();
    Ok(())
}
