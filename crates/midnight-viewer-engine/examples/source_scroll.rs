//! Read-only source/JSON scrolling probe through the production worker and installed plugin.
use midnight_core::*;
use midnight_viewer_engine::{Service, WindowSession};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

static INDEXING_FRAMES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static METADATA_REDRAWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn frame(session: &WindowSession, after: u64) -> Result<ViewerFrame, String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame) if frame.revision > after => {
                    if frame.status.contains("indexing") {
                        INDEXING_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if std::env::var_os("JSON_SCROLL_SETTLE").is_some() {
                            continue;
                        }
                    }
                    return Ok(*frame);
                }
                ViewerOutput::Status(status) => eprintln!("Status: {status}"),
                ViewerOutput::ImageUnavailable { message, .. } => return Err(message),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err(format!("Frame timed out after revision {after}"))
}

fn moving_frame(session: &WindowSession, previous: &ViewerFrame) -> Result<ViewerFrame, String> {
    let mut after = previous.revision;
    loop {
        let candidate = frame(session, after)?;
        after = candidate.revision;
        if candidate.position != previous.position
            || candidate.accessible_text != previous.accessible_text
        {
            return Ok(candidate);
        }
        METADATA_REDRAWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn save(frame: &ViewerFrame, path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write as _;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path.with_extension("ppm"))?);
    write!(file, "P6\n{} {}\n255\n", frame.width, frame.height)?;
    for pixel in frame.pixels.as_chunks::<4>().0 {
        file.write_all(&pixel[..3])?;
    }
    file.flush()?;
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
    let path = PathBuf::from(
        arguments
            .next()
            .ok_or("usage: source_scroll file output-directory [step-pixels [width [height]]]")?,
    );
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
            find: None,
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
    if current.mode != midnight_plugin_host::api::viewer::MODE_SOURCE {
        return Err("The fixture did not open in the installed source viewer".into());
    }
    let highlighted = current
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| {
            pixel[1] > pixel[0].saturating_add(20) && pixel[1] > pixel[2].saturating_add(20)
        })
        .count();
    if highlighted < 20 {
        return Err("No highlighted JSON string pixels in this fixture".into());
    }
    println!(
        "SOURCE_MODE={}, HIGHLIGHTED_PIXELS={highlighted}",
        current.mode
    );
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
                // Locate may repaint the previous camera before this wheel
                // command runs. Revision alone is not a movement receipt. Keep
                // the first moving frame, including any genuine gutter defect.
                let next = moving_frame(&session, &current)?;
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
    if std::env::var_os("JSON_SCROLL_FRACTIONAL").is_some() {
        let reference = current.pixels.clone();
        let mut fractional = Vec::new();
        for _ in 0..120 {
            for delta in [0.25, -0.25] {
                let start = Instant::now();
                if !session.send(ViewerInput::Scroll { x: 0.0, y: delta }) {
                    return Err("fractional scroll queue refused".into());
                }
                current = moving_frame(&session, &current)?;
                fractional.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            if current.pixels != reference {
                return Err("Fractional down/up changed returned pixels".into());
            }
        }
        fractional.sort_by(f64::total_cmp);
        println!(
            "FRACTIONAL: {} frames, p50={:.2}ms p95={:.2}ms max={:.2}ms",
            fractional.len(),
            fractional[fractional.len() / 2],
            fractional[fractional.len() * 95 / 100],
            fractional[fractional.len() - 1]
        );
    }
    std::fs::write(output.join("scroll.csv"), reports.join("\n"))?;
    let mut reference = std::collections::BTreeMap::new();
    let mut sequence = current.position_sequence[0];
    for round in 0..2 {
        for point in [0_u8, 1, 2, 3, 4, 3, 2, 1, 0] {
            sequence += 1;
            if !session.send(ViewerInput::Position {
                generation: current.generation,
                sequence,
                horizontal: false,
                position: f64::from(point) / 4.0,
            }) {
                return Err("scrollbar queue refused".into());
            }
            loop {
                current = frame(&session, current.revision)?;
                if current.position_sequence[0] >= sequence {
                    break;
                }
            }
            if let Some(previous) = reference.get(&point) {
                let previous: &ViewerFrame = previous;
                if previous.pixels != current.pixels {
                    save(
                        previous,
                        output.join(format!("drag-reference-mismatch-{point}")),
                    )?;
                    save(&current, output.join(format!("drag-mismatch-{point}")))?;
                    std::fs::write(
                        output.join("drag-mismatch.txt"),
                        format!(
                            "Reference: {}\nposition={} sequence={:?}\nActual: {}\nposition={} sequence={:?}\ntext_equal={}\n",
                            previous.status,
                            previous.position,
                            previous.position_sequence,
                            current.status,
                            current.position,
                            current.position_sequence,
                            previous.accessible_text == current.accessible_text,
                        ),
                    )?;
                    return Err(
                        format!("Scroll position {point}/4 differs after round {round}").into(),
                    );
                }
            } else {
                save(&current, output.join(format!("drag-reference-{point}")))?;
                reference.insert(point, current.clone());
            }
        }
    }
    println!(
        "INDEXING_FRAMES={}",
        INDEXING_FRAMES.load(std::sync::atomic::Ordering::Relaxed)
    );
    println!(
        "METADATA_REDRAWS={}",
        METADATA_REDRAWS.load(std::sync::atomic::Ordering::Relaxed)
    );
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
