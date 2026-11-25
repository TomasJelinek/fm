//! F3 on every viewable fixture file through the installed viewer plugins: each viewer
//! opens within a time budget and shows the file's content (text, highlighted source,
//! rendered Markdown, HTML with its local stylesheet and image, PDF pages, image pixels,
//! a hex view), and a bad file gives a clear error instead of a hang (damaged images: the
//! host's sentence followed by the decoder's own reason). The open latency
//! of every file is recorded; opens slower than 500 ms are flagged, opens slower than
//! 10 s fail (with the time measured up to two minutes). Two small files are then
//! opened 15 times each, to show the spread of the open time.
//!
//! Generated files in the right panel then cover every extension the Source viewer
//! claims (each must open in Source mode with syntax colours; CSV and TSV with their
//! rainbow columns), the other Markdown, HTML and image extensions (BMP, ICO, TIFF,
//! `.jpeg`), and damaged files of every kind: truncated and garbage PDFs, truncated
//! JPEG, GIF, WebP, BMP and TIFF, a broken SVG (each a clear "damaged or incomplete"
//! refusal), invalid UTF-8 and UTF-16 (shown with replacement characters), NUL bytes
//! (the hex view), broken Markdown, HTML and JSON (shown as far as they go).
//!
//! SVG features are drawn (a marker, a pattern, a mask, a clip path, a blur filter, a
//! `data:` PNG and a `switch` fallback, each proven by its pixels), while a script and
//! images linked to a local file and a URL neither run nor show; hostile SVGs (no SVG
//! namespace, XML entities, nesting past 64 levels, a reference to itself) are refused
//! with the decoder's sentence. Two-frame GIF and WebP files show their first frame and
//! hold still. The licensed AVIF fixtures open: `draw_points_idat.avif` matches its
//! PNG reference pixel for pixel (within the quality-100 encoding's tolerance) at a
//! typed 800 % zoom, and the white pixel is white.
//!
//! An HTML case is skipped, with its reason in the scenario's message, only when the
//! stage has no HTML viewer package, or the HTML viewer reports that this machine
//! refuses its browser: AppArmor denies Chromium's sandbox to this installation (no
//! profile covers the stage), or there is no writable cgroup-v2 delegation (a
//! container). Any other viewer error fails.
use crate::{
    Context,
    app::{App, Config},
    input, panel, viewers_stage,
    viewers_support::{
        self as support, Canvas, Failures, SLOW_OPEN, Viewing, clip, generated, millis, probe,
    },
};
use midnight_core::PaneId;
use slint::{Model, platform::Key};
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(25 * 60);
/// Longer than this without content is a hang, not a slow open.
const OPEN_LIMIT: Duration = Duration::from_secs(120);
/// A viewer that needs longer than this to show its first page fails the scenario.
const OPEN_BUDGET: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
enum Expect {
    /// The visible text contains every fragment.
    Text(&'static [&'static str]),
    /// Opens without content or error.
    Empty,
    /// Source mode, the fragment, and syntax colours a plain text view lacks.
    Source(&'static str),
    /// Rendered Markdown containing the fragment, without its markup.
    Markdown(&'static str, &'static str),
    /// The HTML viewer with the local stylesheet (heading colour) and local image.
    Html,
    /// Plain HTML text.
    HtmlText,
    /// A rendered PDF page.
    Pdf,
    /// Image pixels: at least this many distinct colours, or these colours present.
    Image(usize, &'static [[u8; 3]]),
    /// A hex view showing these leading bytes (hex digits, spaces ignored).
    Hex(&'static str),
    /// A clear refusal, never a hang: it starts with this sentence and goes on with the
    /// decoder's own reason (the image worker protocol's failure reason).
    Rejected(&'static str),
    /// Source mode with the fragment and at least five text colours: the CSV and TSV
    /// columns' rainbow (the first column in the text colour, four column hues).
    Rainbow(&'static str),
    /// A damaged image or document ("image" or "document"): "This <noun> is damaged or
    /// incomplete" with the decoder's reason, or ", or its format is not supported.".
    Damaged(&'static str),
    /// Text with invalid encoding: the fragments, and the invalid bytes made visible
    /// (`\xFF` and `\uD800` escapes, as the basic viewer documents, or U+FFFD).
    Garbled(&'static [&'static str]),
    /// NUL bytes: the hex view with these leading bytes, or text with visible `\x00`.
    Nul(&'static str),
    /// A truncated document or image ("document" or "image"): what can be decoded is
    /// shown, or the file is refused like [`Expect::Damaged`]; never an error or hang.
    Truncated(&'static str),
    /// An image whose colours cover these pixel counts once it holds still (a feature
    /// drawn, content left out, the first frame of an animation).
    Counts(&'static [generated::Count]),
    /// A refusal with this sentence (the decoder's own, shown as it is).
    Statement(&'static str),
}

struct Case {
    /// The left panel shows the fixture (`folder/file`), the right one the generated
    /// files (`file`).
    pane: PaneId,
    path: String,
    kind: String,
    expect: Expect,
}

fn case(path: &str, kind: &str, expect: Expect) -> Case {
    Case {
        pane: PaneId::Left,
        path: path.to_owned(),
        kind: kind.to_owned(),
        expect,
    }
}

fn generated_case(name: &str, kind: &str, expect: Expect) -> Case {
    Case {
        pane: PaneId::Right,
        path: name.to_owned(),
        kind: kind.to_owned(),
        expect,
    }
}

const UTF8_SAMPLE: &[&str] = &["Příliš žluťoučký kůň", "Ελληνικά", "日本語"];
/// The image host's refusal; the decoder's reason follows it.
const DAMAGED_IMAGE: &str = "This image is damaged or incomplete: ";

fn cases() -> Vec<Case> {
    vec![
        case("text/plain.txt", "text", Expect::Text(&["0000 ", "0001 "])),
        case("text/utf8.txt", "text UTF-8", Expect::Text(UTF8_SAMPLE)),
        case(
            "text/utf8-bom.txt",
            "text UTF-8 BOM",
            Expect::Text(UTF8_SAMPLE),
        ),
        case(
            "text/utf16le.txt",
            "text UTF-16 LE",
            Expect::Text(UTF8_SAMPLE),
        ),
        case(
            "text/utf16be.txt",
            "text UTF-16 BE",
            Expect::Text(UTF8_SAMPLE),
        ),
        case(
            "text/crlf.txt",
            "text CRLF",
            Expect::Text(&["0000 ", "0001 "]),
        ),
        case(
            "text/long-line.txt",
            "text 200,000-character line",
            Expect::Text(&["abcdefghijklmnopqrstuvwxyzabcdef"]),
        ),
        case(
            "text/huge.log",
            "text large log",
            Expect::Text(&["2026-11-19T00:00:00Z INFO line=0 "]),
        ),
        case("text/empty.txt", "text empty", Expect::Empty),
        case(
            "text/no-final-newline.txt",
            "text no final newline",
            Expect::Text(&["last line"]),
        ),
        case("code/main.rs", "code Rust", Expect::Source("fn main()")),
        case("code/script.py", "code Python", Expect::Source("def greet")),
        case(
            "code/app.js",
            "code JavaScript",
            Expect::Source("export function sum"),
        ),
        case(
            "code/style.css",
            "code CSS",
            Expect::Source("background: #101820;"),
        ),
        case("code/query.sql", "code SQL", Expect::Source("SELECT name")),
        case("code/build.sh", "code shell", Expect::Source("set -eu")),
        // Routed by its whole name (`makefile` in the Source viewer's manifest).
        case(
            "code/Makefile",
            "code Makefile",
            Expect::Source("@echo done"),
        ),
        case("code/config.json", "code JSON", Expect::Source("\"theme\"")),
        case("code/data.xml", "code XML", Expect::Source("<root>")),
        case(
            "code/settings.toml",
            "code TOML",
            Expect::Source("[window]"),
        ),
        case("code/notes.yaml", "code YAML", Expect::Source("fixture:")),
        case(
            "docs/README.md",
            "Markdown",
            Expect::Markdown("Fixture README", "# Fixture README"),
        ),
        case(
            "docs/guide.md",
            "Markdown table",
            Expect::Markdown("Guide", "| ---"),
        ),
        case("docs/page.html", "HTML with local assets", Expect::Html),
        case("docs/plain.htm", "HTML minimal", Expect::HtmlText),
        case("docs/report.pdf", "PDF", Expect::Pdf),
        case("images/photo.png", "image PNG", Expect::Image(400, &[])),
        case("images/banner.jpg", "image JPEG", Expect::Image(8, &[])),
        case("images/icon.gif", "image GIF", Expect::Image(6, &[])),
        case(
            "images/pattern.webp",
            "image WebP",
            Expect::Image(0, &[[40, 60, 90], [230, 200, 120]]),
        ),
        case(
            "images/vector.svg",
            "image SVG",
            Expect::Image(0, &[[0x20, 0x30, 0x40], [0x3a, 0x7b, 0xd5]]),
        ),
        case(
            "images/truncated.png",
            "image truncated",
            Expect::Rejected(DAMAGED_IMAGE),
        ),
        case(
            "images/zero.png",
            "image zero bytes",
            Expect::Rejected(DAMAGED_IMAGE),
        ),
        case(
            "archives/bundle.zip",
            "archive ZIP",
            Expect::Hex("504B0304"),
        ),
        case(
            "archives/source.tar.gz",
            "archive tar.gz",
            Expect::Hex("1F8B"),
        ),
        case(
            "archives/not-a-zip.zip",
            "text named .zip",
            Expect::Text(&["plain text"]),
        ),
        // One printable byte (0x5A) is detected as text.
        case("sizes/one.bin", "binary one byte", Expect::Text(&["Z"])),
        case("sizes/block.bin", "binary 4 KiB", Expect::Hex("")),
        case("sizes/huge.bin", "binary large", Expect::Hex("")),
        case("sizes/zero.bin", "binary empty", Expect::Empty),
    ]
}

/// Files written into the right panel's folder: every Source extension, the other
/// Markdown, HTML and image extensions, damaged files.
fn generated_cases(root: &std::path::Path) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    for (name, content, fragment) in generated::source_samples() {
        generated::write(root, &name, content)?;
        let expect = if name.ends_with(".csv") || name.ends_with(".tsv") {
            Expect::Rainbow(fragment)
        } else {
            Expect::Source(fragment)
        };
        cases.push(generated_case(&name, "Source extension", expect));
    }
    for name in ["notes.markdown", "notes.mdown", "notes.mkd"] {
        generated::write(root, name, "# Variant heading\n\nText of the variant.\n")?;
        cases.push(generated_case(
            name,
            "Markdown extension",
            Expect::Markdown("Variant heading", "# Variant"),
        ));
    }
    generated::write(
        root,
        "page.xhtml",
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\">\
         <head><title>XHTML</title></head><body><h1>XHTML page</h1><p>Text.</p></body></html>\n",
    )?;
    cases.push(generated_case(
        "page.xhtml",
        "HTML extension",
        Expect::HtmlText,
    ));
    generated::write(
        root,
        "picture.bmp",
        generated::bmp(64, 48, generated::picture),
    )?;
    cases.push(generated_case(
        "picture.bmp",
        "image BMP",
        Expect::Image(50, &[]),
    ));
    generated::write(root, "picture.ico", generated::ico(32, generated::picture))?;
    cases.push(generated_case(
        "picture.ico",
        "image ICO",
        Expect::Image(30, &[]),
    ));
    for name in ["picture.tif", "picture.tiff"] {
        generated::write(root, name, generated::tiff(40, 30, generated::picture))?;
        cases.push(generated_case(name, "image TIFF", Expect::Image(30, &[])));
    }
    generated::write(
        root,
        "photo.jpeg",
        crate::encode::jpeg(16, 8, |x, y| ((x * 15 + y * 8) % 256) as u8),
    )?;
    cases.push(generated_case(
        "photo.jpeg",
        "image .jpeg",
        Expect::Image(8, &[]),
    ));
    generated::write(root, "linked.png", generated::linked_png())?;
    // The licensed AVIF fixtures and their reference, checked together (`avif`).
    generated::write(root, "draw_points.png", generated::POINTS_PNG)?;
    generated::write(root, "draw_points_idat.avif", generated::POINTS_AVIF)?;
    generated::write(root, "white_1x1.avif", generated::WHITE_AVIF)?;
    for feature in generated::svg_features() {
        generated::write(root, feature.name, &feature.text)?;
        cases.push(generated_case(
            feature.name,
            "SVG feature",
            Expect::Counts(feature.counts),
        ));
    }
    for (name, text, sentence) in generated::hostile_svgs() {
        generated::write(root, name, text)?;
        cases.push(generated_case(
            name,
            "hostile SVG",
            Expect::Statement(sentence),
        ));
    }
    // 48 x 48 frames: 2,304 pixels of the first colour, none of the second.
    const FIRST: [u8; 3] = [0x20, 0x80, 0xff];
    const SECOND: [u8; 3] = [0xff, 0x80, 0x20];
    const FIRST_FRAME: &[generated::Count] = &[(FIRST, 2200, 2304), (SECOND, 0, 0)];
    generated::write(
        root,
        "two-frames.gif",
        generated::two_frame_gif(FIRST, SECOND),
    )?;
    cases.push(generated_case(
        "two-frames.gif",
        "animated GIF",
        Expect::Counts(FIRST_FRAME),
    ));
    generated::write(
        root,
        "two-frames.webp",
        generated::two_frame_webp(FIRST, SECOND),
    )?;
    cases.push(generated_case(
        "two-frames.webp",
        "animated WebP",
        Expect::Counts(FIRST_FRAME),
    ));
    for damaged in generated::damaged() {
        generated::write(root, damaged.name, &damaged.bytes)?;
        let expect = match damaged.name {
            "truncated.pdf" => Expect::Truncated("document"),
            "truncated.jpg" => Expect::Truncated("image"),
            "garbage.pdf" | "empty.pdf" => Expect::Damaged("document"),
            "invalid-utf8.txt" => Expect::Garbled(&["valid start", "valid end"]),
            "odd-utf16.txt" => Expect::Garbled(&["hi"]),
            "nul.txt" => Expect::Nul("7465787400"),
            "broken.md" => Expect::Markdown("Broken heading", "# Broken"),
            "broken.html" => Expect::HtmlText,
            "broken.json" => Expect::Source("[1, 2,,"),
            _ => Expect::Damaged("image"),
        };
        cases.push(generated_case(damaged.name, damaged.kind, expect));
    }
    Ok(cases)
}

pub fn every_kind(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let kinds = context.work.join("kinds");
    std::fs::create_dir_all(&kinds).map_err(|error| error.to_string())?;
    let generated = generated_cases(&kinds)?;
    let mut config = Config::new(&root, &kinds);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let failures = Failures::default();
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    let mut folder = String::new();
    for case in cases().into_iter().chain(generated) {
        let name = if case.pane == PaneId::Left {
            let (parent, name) = case.path.split_once('/').expect("folder/file");
            if folder != parent {
                if !folder.is_empty() {
                    panel::leave(&mut app, PaneId::Left)?;
                }
                panel::enter(&mut app, PaneId::Left, parent)?;
                folder = parent.to_owned();
            }
            name.to_owned()
        } else {
            case.path.clone()
        };
        if let Err(error) = panel::select_row(&mut app, case.pane, &name) {
            failures.check(&case.path, Err(error));
            continue;
        }
        let outcome = open_and_check(&mut app, context, &case);
        match outcome {
            Ok((latency, window_time, status, shot)) => {
                failures.check(
                    &case.path,
                    if latency > OPEN_BUDGET {
                        Err(format!(
                            "the first content took {} (budget {})",
                            millis(latency),
                            millis(OPEN_BUDGET)
                        ))
                    } else {
                        Ok(())
                    },
                );
                if latency > SLOW_OPEN {
                    context.outcome.flags.push(crate::report::Flag {
                        shot: shot.clone(),
                        kind: "slow viewer open",
                        detail: format!(
                            "{} ({}) took {} to show its content (budget {})",
                            case.path,
                            case.kind,
                            millis(latency),
                            millis(SLOW_OPEN)
                        ),
                    });
                    if let Some(entry) = context
                        .outcome
                        .shots
                        .iter_mut()
                        .find(|entry| entry.file == shot)
                    {
                        entry.flags += 1;
                    }
                }
                rows.push(format!(
                    "{}\t{}\t{}\t{}\t{}",
                    case.kind,
                    case.path,
                    latency.as_millis(),
                    window_time.as_millis(),
                    clip(&status, 80)
                ));
            }
            Err(error) => match error.strip_prefix("SKIP:") {
                Some(reason) => {
                    skipped.push(format!("{} ({}): {}", case.path, case.kind, reason.trim()))
                }
                None => failures.check(&case.path, Err(error)),
            },
        }
    }
    failures.check(
        "AVIF fixtures (draw_points_idat.avif against draw_points.png, white_1x1.avif)",
        avif(&mut app, context),
    );
    skipped.push(format!(
        "the HTML viewer's AppArmor Install… offer (O20): {}",
        apparmor_offer()
    ));
    if !skipped.is_empty() {
        let text = format!("SKIP {} case(s): {}", skipped.len(), skipped.join("; "));
        context.note(text.clone());
        context.outcome.message = text;
    }
    context.note(format!(
        "Open latency (F3 to the viewer's first content; debug test build, software \
         rendering), kind / file / total ms / of which window creation ms / status:\n{}",
        rows.join("\n")
    ));
    for row in &rows {
        context.note(format!("latency\t{row}"));
    }
    // The same small files opened again and again: the spread of the open time.
    panel::focus_list(&mut app, PaneId::Left)?;
    panel::leave(&mut app, PaneId::Left)?;
    for (folder, name) in [("text", "no-final-newline.txt"), ("sizes", "block.bin")] {
        panel::enter(&mut app, PaneId::Left, folder)?;
        failures.check(
            &format!("{folder}/{name} repeatedly"),
            repeat(&mut app, context, folder, name),
        );
        panel::leave(&mut app, PaneId::Left)?;
    }
    app.shutdown()?;
    failures.into_result()
}

/// Why the HTML viewer's AppArmor Install… offer (shown when AppArmor refuses
/// Chromium's sandbox to an installation no profile covers) is not checked here.
fn apparmor_offer() -> &'static str {
    if cfg!(windows) {
        "Windows has no AppArmor; the offer exists on Linux only"
    } else if std::fs::read_to_string("/sys/module/apparmor/parameters/enabled")
        .is_ok_and(|enabled| enabled.trim() == "Y")
    {
        "needs Ubuntu outside WSL that restricts unprivileged user namespaces and a stage \
         outside the profile's pattern; no such machine in this run"
    } else {
        "AppArmor is not enabled on this machine (WSL); the offer needs Ubuntu outside WSL"
    }
}

/// Open and close one file `REPEATS` times; note the distribution and flag slow opens.
fn repeat(app: &mut App, context: &mut Context, folder: &str, name: &str) -> Result<(), String> {
    const REPEATS: usize = 15;
    let mut times = Vec::with_capacity(REPEATS);
    for _ in 0..REPEATS {
        panel::select_row(app, PaneId::Left, name)?;
        let viewing = support::view(app, false, OPEN_LIMIT)?;
        times.push(viewing.latency);
        let result = viewing.result.clone();
        viewing.close(app)?;
        result?;
    }
    let mut sorted = times.clone();
    sorted.sort();
    let slow = times.iter().filter(|time| **time > SLOW_OPEN).count();
    context.note(format!(
        "latency\trepeated opens\t{folder}/{name}\t{}\tmin {} / median {} / max {}; {slow} of \
         {REPEATS} over {}; in order: {:?}",
        sorted[REPEATS / 2].as_millis(),
        millis(sorted[0]),
        millis(sorted[REPEATS / 2]),
        millis(sorted[REPEATS - 1]),
        millis(SLOW_OPEN),
        times
            .iter()
            .map(|time| time.as_millis())
            .collect::<Vec<_>>()
    ));
    if slow > 0 {
        let shot = app.screenshot(
            context,
            &format!("repeated-{}", name.replace('.', "-")),
            &format!("After {REPEATS} opens of {folder}/{name}"),
        );
        context.outcome.flags.push(crate::report::Flag {
            shot,
            kind: "slow viewer open",
            detail: format!(
                "{slow} of {REPEATS} opens of {folder}/{name} took over {} (max {})",
                millis(SLOW_OPEN),
                millis(sorted[REPEATS - 1])
            ),
        });
        if let Some(entry) = context.outcome.shots.last_mut() {
            entry.flags += 1;
        }
    }
    Ok(())
}

/// Open the selected file, verify it, screenshot and close the viewer.
fn open_and_check(
    app: &mut App,
    context: &mut Context,
    case: &Case,
) -> Result<(Duration, Duration, String, String), String> {
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let (latency, window_time) = (viewing.latency, viewing.window_time);
    let checked = check(app, &viewing, case);
    let status = viewing.status();
    let shot_name = case.path.replace(['/', '.'], "-");
    let shot = app.screenshot_window(
        context,
        &viewing.software,
        &shot_name,
        &format!(
            "F3 {} ({}): {} · {}",
            case.path,
            case.kind,
            millis(latency),
            clip(&status, 100)
        ),
    );
    // The UI review's layout checks on every viewer kind's window (flags, not failures).
    let findings = probe::review(context, viewing.slint(), &shot);
    if !findings.is_empty() {
        context.note(format!(
            "{}: layout review: {}",
            case.path,
            findings.join("; ")
        ));
    }
    context.note(format!(
        "{}: {} in {}; status {:?}; text {:?}",
        case.path,
        match &viewing.result {
            Ok(()) => "content".to_owned(),
            Err(error) => format!("error {error:?}"),
        },
        millis(latency),
        status,
        clip(&viewing.text(), 120)
    ));
    let closed = viewing.close(app);
    checked?;
    closed?;
    Ok((latency, window_time, status, shot))
}

fn check(app: &mut App, viewing: &Viewing, case: &Case) -> Result<(), String> {
    let window = &viewing.window;
    if matches!(case.expect, Expect::Html | Expect::HtmlText)
        && let Some(reason) = probe::html_unavailable()
    {
        return Err(format!("SKIP: {reason}"));
    }
    if !matches!(
        case.expect,
        Expect::Rejected(_) | Expect::Damaged(_) | Expect::Truncated(_) | Expect::Statement(_)
    ) && let Err(error) = &viewing.result
    {
        if matches!(case.expect, Expect::Html | Expect::HtmlText)
            && let Some(reason) = support::browser_refused(error)
        {
            return Err(format!("SKIP: {reason}: {error:?}"));
        }
        return Err(format!(
            "the viewer reported {error:?}{}",
            if error.contains("teardown is unconfirmed") {
                " (finding V1: the viewer engine's local-resource scope has no \
                 release_on_worker, content_resources.rs)"
            } else {
                ""
            }
        ));
    }
    match case.expect {
        Expect::Text(fragments) => {
            viewing.wait(app, "the text", Duration::from_secs(10), |window| {
                let text = window.get_accessible_text();
                fragments.iter().all(|fragment| text.contains(fragment))
            })?;
            let text = viewing.text();
            if text.contains('\u{feff}') {
                return Err("the text shows the byte order mark".into());
            }
            if case.path.ends_with("crlf.txt") && text.contains('\r') {
                return Err("CRLF line ends show a carriage return".into());
            }
            Ok(())
        }
        Expect::Empty => {
            // Settled: the page holds still, so no later content can come.
            probe::still_canvas(app, viewing, Duration::from_secs(10))?;
            let text = viewing.text();
            if !text.trim().is_empty() {
                return Err(format!("an empty file shows {:?}", clip(&text, 80)));
            }
            Ok(())
        }
        Expect::Source(fragment) => {
            viewing.wait(
                app,
                "highlighted source",
                Duration::from_secs(10),
                |window| window.get_accessible_text().contains(fragment),
            )?;
            if !window.get_source_mode() {
                return Err(format!(
                    "not shown in source mode (modes {:?})",
                    window.get_modes().iter().collect::<Vec<_>>()
                ));
            }
            let accents = off_text_pixels(&viewing.canvas(), viewing);
            if accents < 40 {
                return Err(format!(
                    "no syntax colours: only {accents} pixels differ from plain text colours"
                ));
            }
            Ok(())
        }
        Expect::Markdown(fragment, markup) => {
            viewing.wait(
                app,
                "rendered Markdown",
                Duration::from_secs(10),
                |window| window.get_accessible_text().contains(fragment),
            )?;
            if !window.get_markdown_mode() {
                return Err("not shown in Markdown mode".into());
            }
            if viewing.text().contains(markup) {
                return Err(format!("the Markdown markup {markup:?} is shown raw"));
            }
            Ok(())
        }
        Expect::Html => {
            if !window.get_web_mode() {
                return Err(format!(
                    "not shown by the HTML viewer (status {:?})",
                    viewing.status()
                ));
            }
            // The local stylesheet colours the heading #3a7bd5; the local image is a
            // 160x120 gradient with thousands of colours.
            let result = viewing.wait(app, "the styled page", Duration::from_secs(15), |_| {
                let canvas = viewing.canvas();
                canvas.count_near([0x3a, 0x7b, 0xd5], 24) >= 40 && canvas.colours(1).len() > 1500
            });
            if result.is_err() {
                let canvas = viewing.canvas();
                return Err(format!(
                    "the page lacks its stylesheet or image: {} heading-colour pixels, {} colours",
                    canvas.count_near([0x3a, 0x7b, 0xd5], 24),
                    canvas.colours(1).len()
                ));
            }
            Ok(())
        }
        Expect::HtmlText => {
            if !window.get_web_mode() {
                return Err("not shown by the HTML viewer".into());
            }
            viewing.wait(app, "the page", Duration::from_secs(15), |_| {
                viewing.canvas().colours(4).len() >= 2
            })
        }
        Expect::Pdf => {
            viewing.wait(app, "the PDF page", Duration::from_secs(15), |window| {
                window.get_document_pages() >= 1
            })?;
            let canvas = viewing.canvas();
            let bright = canvas.count_near([255, 255, 255], 8);
            let dark = canvas
                .pixels
                .iter()
                .filter(|pixel| pixel.r < 90 && pixel.g < 90 && pixel.b < 90)
                .count();
            if bright < 1000 || dark < 20 {
                return Err(format!(
                    "the page is not rendered: {bright} white and {dark} dark pixels"
                ));
            }
            Ok(())
        }
        Expect::Image(distinct, colours) => {
            if !window.get_image_mode() {
                return Err(format!(
                    "not shown as an image (status {:?})",
                    viewing.status()
                ));
            }
            let result = viewing.wait(app, "the image pixels", Duration::from_secs(10), |_| {
                let canvas = viewing.canvas();
                canvas.colours(1).len() >= distinct
                    && colours
                        .iter()
                        .all(|colour| canvas.count_near(*colour, 6) >= 30)
            });
            if result.is_err() {
                let canvas = viewing.canvas();
                return Err(format!(
                    "the image is not shown: {} colours (want {distinct}), wanted colours {:?} \
                     found {:?} times",
                    canvas.colours(1).len(),
                    colours,
                    colours
                        .iter()
                        .map(|colour| canvas.count_near(*colour, 6))
                        .collect::<Vec<_>>()
                ));
            }
            Ok(())
        }
        Expect::Hex(leading) => {
            viewing.wait(app, "the hex view", Duration::from_secs(10), |window| {
                let text: String = window
                    .get_accessible_text()
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect::<String>()
                    .to_uppercase();
                !text.is_empty() && text.contains(leading)
            })
        }
        Expect::Rejected(sentence) => {
            let reason = |viewing: &Viewing| {
                [viewing.result.clone().err(), Some(viewing.status())]
                    .into_iter()
                    .flatten()
                    .find_map(|text| {
                        text.split_once(sentence)
                            .map(|(_, reason)| reason.trim().to_owned())
                            .filter(|reason| !reason.is_empty())
                    })
            };
            let _ = viewing.wait(app, "the refusal", Duration::from_secs(10), |_| {
                reason(viewing).is_some()
            });
            let status = viewing.status();
            if reason(viewing).is_none() {
                return Err(format!(
                    "no refusal with the decoder's reason (expected {sentence:?} and a \
                     reason): result {:?}, status {status:?}",
                    viewing.result
                ));
            }
            Ok(())
        }
        Expect::Rainbow(fragment) => {
            viewing.wait(app, "the table", Duration::from_secs(10), |window| {
                window.get_accessible_text().contains(fragment)
            })?;
            if !window.get_source_mode() {
                return Err("a table is not shown in source mode".into());
            }
            let canvas = probe::still_canvas(app, viewing, Duration::from_secs(10))?;
            let colours = probe::glyph_colours(&canvas, probe::dominant(&canvas), 24);
            if colours.len() < 5 {
                return Err(format!(
                    "the columns show {} text colour(s), expected the text colour and four \
                     column hues: {:?}",
                    colours.len(),
                    colours
                        .iter()
                        .map(|(colour, pixels)| format!("{} ({pixels} px)", probe::hex(*colour)))
                        .collect::<Vec<_>>()
                ));
            }
            Ok(())
        }
        Expect::Damaged(noun) => {
            let sentence = format!("This {noun} is damaged or incomplete");
            let answered = |viewing: &Viewing| {
                [viewing.result.clone().err(), Some(viewing.status())]
                    .into_iter()
                    .flatten()
                    .find_map(|text| {
                        let (_, rest) = text.split_once(&sentence)?;
                        let reason = rest
                            .strip_prefix(':')
                            .map(str::trim)
                            .filter(|reason| !reason.is_empty())
                            .map(str::to_owned);
                        reason.or_else(|| {
                            rest.starts_with(", or its format is not supported")
                                .then(|| "format not supported".to_owned())
                        })
                    })
            };
            let _ = viewing.wait(app, "the refusal", Duration::from_secs(10), |_| {
                answered(viewing).is_some()
            });
            let status = viewing.status();
            if answered(viewing).is_none() {
                return Err(format!(
                    "no clear refusal (expected {sentence:?} with a reason): result {:?}, \
                     status {status:?}",
                    viewing.result
                ));
            }
            Ok(())
        }
        Expect::Garbled(fragments) => {
            viewing.wait(app, "the text", Duration::from_secs(10), |window| {
                let text = window.get_accessible_text();
                fragments.iter().all(|fragment| text.contains(fragment))
            })?;
            let text = viewing.text();
            if !(text.contains('\u{fffd}') || text.contains("\\x") || text.contains("\\u")) {
                return Err(format!(
                    "invalid bytes are not made visible: {:?}",
                    clip(&text, 120)
                ));
            }
            Ok(())
        }
        Expect::Nul(leading) => {
            viewing.wait(app, "the bytes", Duration::from_secs(10), |window| {
                let text = window.get_accessible_text();
                let compact: String = text
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect::<String>()
                    .to_uppercase();
                compact.contains(leading) || text.contains("\\x00")
            })?;
            if viewing.text().contains('\0') {
                return Err("NUL bytes are shown raw".into());
            }
            Ok(())
        }
        Expect::Truncated(noun) => {
            let sentence = format!("This {noun} is damaged or incomplete");
            let refused = |viewing: &Viewing| {
                [viewing.result.clone().err(), Some(viewing.status())]
                    .into_iter()
                    .flatten()
                    .any(|text| text.contains(&sentence))
            };
            let shown = |viewing: &Viewing| {
                let window = &viewing.window;
                window.get_image_mode()
                    && viewing.canvas().colours(4).len() >= 2
                    && (noun != "document" || window.get_document_pages() >= 1)
            };
            // Either answer settles it: a refusal, or what could be decoded, held still.
            let _ = viewing.wait(
                app,
                "the refusal or the picture",
                Duration::from_secs(10),
                |_| refused(viewing) || shown(viewing),
            );
            if refused(viewing) {
                return Ok(());
            }
            if let Err(error) = &viewing.result {
                return Err(format!("neither shown nor clearly refused: {error:?}"));
            }
            probe::still_canvas(app, viewing, Duration::from_secs(10))?;
            let status = viewing.status();
            if refused(viewing) {
                return Ok(());
            }
            if !shown(viewing) || support::probe::status_error(&status).is_some() {
                return Err(format!(
                    "neither shown nor clearly refused: status {status:?}, image mode {}",
                    window.get_image_mode()
                ));
            }
            Ok(())
        }
        Expect::Counts(counts) => {
            if !window.get_image_mode() {
                return Err(format!(
                    "not shown as an image (status {:?})",
                    viewing.status()
                ));
            }
            let measure = |canvas: &Canvas| -> Vec<usize> {
                counts
                    .iter()
                    .map(|(colour, _, _)| canvas.count_near(*colour, 6))
                    .collect()
            };
            let within = |found: &[usize]| {
                counts
                    .iter()
                    .zip(found)
                    .all(|((_, least, most), found)| (least..=most).contains(&found))
            };
            let _ = viewing.wait(app, "the drawn image", Duration::from_secs(10), |_| {
                within(&measure(&viewing.canvas()))
            });
            // Held still: an animation that moved on would keep changing the picture.
            let canvas = probe::still_canvas(app, viewing, Duration::from_secs(10))?;
            let found = measure(&canvas);
            if !within(&found) {
                return Err(format!(
                    "the picture shows {} (colour: pixels found, wanted)",
                    counts
                        .iter()
                        .zip(&found)
                        .map(|((colour, least, most), found)| format!(
                            "{}: {found}, {least}–{most}",
                            probe::hex(*colour)
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            Ok(())
        }
        Expect::Statement(sentence) => {
            let refused = |viewing: &Viewing| {
                [viewing.result.clone().err(), Some(viewing.status())]
                    .into_iter()
                    .flatten()
                    .any(|text| text.contains(sentence))
            };
            let _ = viewing.wait(app, "the refusal", Duration::from_secs(10), |_| {
                refused(viewing)
            });
            if !refused(viewing) {
                return Err(format!(
                    "no refusal saying {sentence:?}: result {:?}, status {:?}",
                    viewing.result,
                    viewing.status()
                ));
            }
            Ok(())
        }
    }
}

/// These small fixtures fit at 100 %. Enlarge each one to inspect individual pixels.
fn zoom_avif_fixture(app: &mut App, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(
        app,
        "the image to open in Fit",
        Duration::from_secs(10),
        |window| window.get_image_mode() && (window.get_zoom_percent() - 100.0).abs() < 0.5,
    )?;
    viewing.click_content(app);
    input::chord(viewing.slint(), &[Key::Control], "y");
    viewing.wait(
        app,
        "Ctrl+Y to open the zoom field",
        Duration::from_secs(10),
        |window| window.get_zoom_editing(),
    )?;
    input::replace_text(viewing.slint(), "800");
    input::key(viewing.slint(), Key::Return);
    viewing.wait(app, "the 800 % zoom", Duration::from_secs(10), |window| {
        !window.get_zoom_editing() && (window.get_zoom_percent() - 800.0).abs() < 0.5
    })
}

/// Compare the decoded AVIF pixels with their PNG reference and check the white pixel.
fn avif(app: &mut App, context: &mut Context) -> Result<(), String> {
    let mut renders = Vec::new();
    for name in ["draw_points.png", "draw_points_idat.avif"] {
        panel::select_row(app, PaneId::Right, name)?;
        let viewing = support::view(app, false, OPEN_LIMIT)?;
        let result: Result<Canvas, String> = (|| {
            viewing.result.clone()?;
            zoom_avif_fixture(app, &viewing)?;
            let canvas = probe::still_canvas(app, &viewing, Duration::from_secs(10))?;
            app.screenshot_window(
                context,
                &viewing.software,
                &format!("avif-{}", name.replace('.', "-")),
                &format!("{name} at 800 %"),
            );
            Ok(canvas)
        })();
        viewing.close(app)?;
        renders.push(result.map_err(|error| format!("{name}: {error}"))?);
    }
    let (png, avif) = (&renders[0], &renders[1]);
    if (png.width, png.height) != (avif.width, avif.height) {
        return Err(format!(
            "the two renders differ in size: {}x{} and {}x{}",
            png.width, png.height, avif.width, avif.height
        ));
    }
    let background = probe::dominant(png);
    let mut image = 0;
    let mut close = 0;
    let mut largest = 0;
    for y in 0..png.height {
        for x in 0..png.width {
            let (a, b) = (png.at(x, y), avif.at(x, y));
            if a == background && b == background {
                continue;
            }
            image += 1;
            let deviation = (0..3)
                .map(|channel| a[channel].abs_diff(b[channel]))
                .max()
                .unwrap_or(0);
            largest = largest.max(deviation);
            if deviation <= 6 {
                close += 1;
            }
        }
    }
    context.note(format!(
        "AVIF: {image} image pixels at 800 %, {close} within 6 of the PNG reference, largest \
         deviation {largest}"
    ));
    // 33 x 11 pixels at 800 %: 23,232 pixels.
    if image < 20_000 || close * 1000 < image * 995 {
        return Err(format!(
            "draw_points_idat.avif does not match draw_points.png: {close} of {image} image \
             pixels within 6 per channel (largest deviation {largest})"
        ));
    }
    panel::select_row(app, PaneId::Right, "white_1x1.avif")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let white = (|| {
        viewing.result.clone()?;
        zoom_avif_fixture(app, &viewing)?;
        viewing.wait(
            app,
            "the white pixel at 800 %",
            Duration::from_secs(10),
            |window| {
                window.get_image_mode()
                    && (window.get_zoom_percent() - 800.0).abs() < 0.5
                    && viewing.canvas().count_near([255, 255, 255], 3) >= 60
            },
        )
    })();
    viewing.click_content(app);
    input::chord(viewing.slint(), &[Key::Control], "0");
    let fitted = viewing.wait(app, "Ctrl+0 to fit", Duration::from_secs(10), |window| {
        window.get_zoom_percent() < 800.0
    });
    viewing.close(app)?;
    white.map_err(|error| format!("white_1x1.avif: {error}"))?;
    fitted
}

/// Pixels clearly off the blend between the text and background colours: syntax
/// colours, which plain text does not have.
fn off_text_pixels(canvas: &Canvas, viewing: &Viewing) -> usize {
    let foreground = viewing.window.get_foreground().to_argb_u8();
    let background = viewing.window.get_background_color().to_argb_u8();
    let (f, b) = (
        [foreground.red, foreground.green, foreground.blue].map(f32::from),
        [background.red, background.green, background.blue].map(f32::from),
    );
    let axis = [f[0] - b[0], f[1] - b[1], f[2] - b[2]];
    let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).max(1.0);
    canvas
        .pixels
        .iter()
        .filter(|pixel| {
            let p = [f32::from(pixel.r), f32::from(pixel.g), f32::from(pixel.b)];
            let offset = [p[0] - b[0], p[1] - b[1], p[2] - b[2]];
            let t = ((offset[0] * axis[0] + offset[1] * axis[1] + offset[2] * axis[2]) / length)
                .clamp(0.0, 1.0);
            let distance = (0..3)
                .map(|channel| (offset[channel] - t * axis[channel]).powi(2))
                .sum::<f32>()
                .sqrt();
            distance > 48.0
        })
        .count()
}
