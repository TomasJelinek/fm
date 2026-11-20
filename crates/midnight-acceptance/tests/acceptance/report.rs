//! Results, screenshots and the HTML/JSON/text report of one run.
use super::{Options, Scenario, fixtures::Fixture};
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Runs kept below `_build/acceptance`; older ones are deleted when a run starts.
const KEPT_RUNS: usize = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Pass,
    /// Passed, but the UI review flagged layout problems.
    Warn,
    Fail,
    Skip,
    Timeout,
    NotRun,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::Skip => "SKIP",
            Self::Timeout => "TIMEOUT",
            Self::NotRun => "NOT RUN",
        }
    }

    pub fn is_failure(self, strict_review: bool) -> bool {
        matches!(self, Self::Fail | Self::Timeout | Self::NotRun)
            || (strict_review && self == Self::Warn)
    }
}

/// One rendered window.
pub struct Shot {
    /// Path relative to the run directory.
    pub file: String,
    pub caption: String,
    pub flags: usize,
}

/// One automatic layout finding of the UI review.
pub struct Flag {
    pub shot: String,
    pub kind: &'static str,
    pub detail: String,
}

#[derive(Default)]
pub struct Outcome {
    pub status: Status,
    pub message: String,
    pub duration: Duration,
    pub notes: Vec<String>,
    pub shots: Vec<Shot>,
    pub flags: Vec<Flag>,
}

impl Outcome {
    pub fn with(status: Status, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            ..Self::default()
        }
    }

    pub fn headline(&self) -> String {
        let mut text = self.message.lines().next().unwrap_or_default().to_owned();
        if !self.flags.is_empty() {
            if !text.is_empty() {
                text.push_str("; ");
            }
            let _ = write!(
                text,
                "{} review flag(s) in {} screenshot(s)",
                self.flags.len(),
                self.shots.len()
            );
        } else if !self.shots.is_empty() && text.is_empty() {
            let _ = write!(text, "{} screenshot(s)", self.shots.len());
        }
        text
    }
}

/// `YYYYMMDD-HHMMSS` in UTC.
pub fn run_id() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let (days, time) = (seconds / 86_400, seconds % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        time / 3_600,
        time / 60 % 60,
        time % 60
    )
}

fn is_run_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 15
        && bytes[8] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit())
}

/// Keep the newest [`KEPT_RUNS`] run directories next to `current` (including it).
/// Only the default location `_build/acceptance` is pruned: a run placed elsewhere
/// with `--run-dir` never deletes its siblings.
pub fn prune_old_runs(current: &Path) {
    let Some(parent) = current.parent() else {
        return;
    };
    if !parent.ends_with("_build/acceptance") {
        return;
    }
    let mut runs: Vec<PathBuf> = fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| entry.file_name().to_str().is_some_and(is_run_name))
        .map(|entry| entry.path())
        .collect();
    runs.sort();
    let excess = runs.len().saturating_sub(KEPT_RUNS);
    for old in runs.into_iter().take(excess) {
        if old != current {
            let _ = super::fixtures::remove_tree(&old);
        }
    }
}

thread_local! {
    static ENCODER: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("PNG encoder runtime");
}

/// Write an RGB frame as a compressed PNG with the application's own snapshot encoder.
pub fn save_png(
    path: &Path,
    width: usize,
    height: usize,
    pixels: &[slint::Rgb8Pixel],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let _ = fs::remove_file(path);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in pixels.iter().take(width * height) {
        rgba.extend_from_slice(&[pixel.r, pixel.g, pixel.b, 255]);
    }
    ENCODER.with(|runtime| {
        runtime.block_on(midnight_engine::snapshot::save(
            path.to_path_buf(),
            width as u32,
            height as u32,
            rgba,
        ))
    })
}

fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn json(text: &str) -> String {
    let mut output = String::with_capacity(text.len() + 2);
    output.push('"');
    for character in text.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                let _ = write!(output, "\\u{:04x}", control as u32);
            }
            other => output.push(other),
        }
    }
    output.push('"');
    output
}

/// Write `index.html`, `summary.json` and `summary.txt`; returns the index path.
pub fn write(
    run_dir: &Path,
    fixture: &Fixture,
    results: &[(&Scenario, Outcome)],
    elapsed: Duration,
    options: &Options,
) -> PathBuf {
    let mut page = String::new();
    let _ = write!(
        page,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>Acceptance run {run}</title><style>\
:root{{--bg:#fff;--fg:#1d2329;--muted:#5d6b78;--line:#d5dbe1;--pass:#1e7d32;--warn:#a36200;--fail:#b3261e;--card:#f5f7f9}}\
@media (prefers-color-scheme: dark){{:root{{--bg:#14181c;--fg:#e3e8ed;--muted:#9aa7b4;--line:#2c343c;--pass:#6fcf7a;--warn:#f0b35a;--fail:#ff8a80;--card:#1c2228}}}}\
body{{background:var(--bg);color:var(--fg);font:14px/1.45 system-ui,sans-serif;margin:0 auto;max-width:1400px;padding:16px}}\
table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid var(--line);padding:4px 8px;text-align:left;vertical-align:top}}\
.PASS{{color:var(--pass)}}.WARN{{color:var(--warn)}}.FAIL,.TIMEOUT,.NOTRUN{{color:var(--fail)}}.SKIP{{color:var(--muted)}}\
.grid{{display:grid;gap:12px;grid-template-columns:repeat(auto-fill,minmax(300px,1fr))}}\
figure{{background:var(--card);border:1px solid var(--line);border-radius:6px;margin:0;padding:8px}}\
figure img{{max-width:100%;height:auto;display:block}}figcaption{{font-size:12px;color:var(--muted);overflow-wrap:anywhere}}\
.flagged{{border-color:var(--warn)}}code{{font-size:12px}}ul.flags{{margin:4px 0;padding-left:18px;font-size:12px}}\
</style></head><body><h1>Acceptance run {run}</h1>",
        run = html(
            &run_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        )
    );
    let failed = results
        .iter()
        .filter(|(_, outcome)| outcome.status.is_failure(options.strict_review))
        .count();
    let _ = write!(
        page,
        "<p>{} scenario(s), {failed} failed, {:.1} s. Fixture: <code>{}</code> ({:?} scale, seed {:#x}). \
Review strictness: {}.</p><table><tr><th>Scenario</th><th>Area</th><th>Result</th><th>Time</th><th>Details</th></tr>",
        results.len(),
        elapsed.as_secs_f64(),
        html(&fixture.root.display().to_string()),
        fixture.scale,
        super::fixtures::SEED,
        if options.strict_review {
            "flags fail the run"
        } else {
            "flags are warnings"
        }
    );
    for (scenario, outcome) in results {
        let label = outcome.status.label();
        let _ = write!(
            page,
            "<tr><td><a href=\"#{name}\">{name}</a></td><td>{area}</td><td class=\"{class}\">{label}</td>\
<td>{:.1} s</td><td>{}</td></tr>",
            outcome.duration.as_secs_f64(),
            html(&outcome.headline()),
            name = scenario.name,
            area = scenario.area,
            class = label.replace(' ', ""),
        );
    }
    page.push_str("</table>");
    for (scenario, outcome) in results {
        let _ = write!(
            page,
            "<h2 id=\"{name}\">{name} <small class=\"{class}\">{label}</small></h2><p>{}</p>",
            html(scenario.summary),
            name = scenario.name,
            class = outcome.status.label().replace(' ', ""),
            label = outcome.status.label()
        );
        if !outcome.message.is_empty() {
            let _ = write!(page, "<pre>{}</pre>", html(&outcome.message));
        }
        if !outcome.notes.is_empty() {
            page.push_str("<ul>");
            for note in &outcome.notes {
                let _ = write!(page, "<li>{}</li>", html(note));
            }
            page.push_str("</ul>");
        }
        if !outcome.shots.is_empty() {
            page.push_str("<div class=\"grid\">");
            for shot in &outcome.shots {
                let flags: Vec<&Flag> = outcome
                    .flags
                    .iter()
                    .filter(|flag| flag.shot == shot.file)
                    .collect();
                let _ = write!(
                    page,
                    "<figure class=\"{}\"><a href=\"{file}\"><img loading=\"lazy\" src=\"{file}\" alt=\"{caption}\"></a>\
<figcaption>{caption}</figcaption>",
                    if flags.is_empty() { "" } else { "flagged" },
                    file = html(&shot.file),
                    caption = html(&shot.caption)
                );
                if !flags.is_empty() {
                    page.push_str("<ul class=\"flags\">");
                    for flag in flags {
                        let _ = write!(
                            page,
                            "<li><b>{}</b>: {}</li>",
                            flag.kind,
                            html(&flag.detail)
                        );
                    }
                    page.push_str("</ul>");
                }
                page.push_str("</figure>");
            }
            page.push_str("</div>");
        }
    }
    page.push_str(
        "<h2>Fixture catalogue</h2><table><tr><th>Path</th><th>Kind</th><th>Notes</th></tr>",
    );
    for item in &fixture.items {
        let _ = write!(
            page,
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            html(&item.path),
            item.kind,
            html(item.note)
        );
    }
    page.push_str("</table></body></html>\n");
    let index = run_dir.join("index.html");
    let _ = fs::write(&index, page);

    let mut summary = String::from("{\n  \"scenarios\": [\n");
    for (position, (scenario, outcome)) in results.iter().enumerate() {
        let _ = writeln!(
            summary,
            "    {{\"name\": {}, \"area\": {}, \"status\": {}, \"seconds\": {:.3}, \"message\": {}, \
\"screenshots\": {}, \"flags\": {}}}{}",
            json(scenario.name),
            json(scenario.area),
            json(outcome.status.label()),
            outcome.duration.as_secs_f64(),
            json(&outcome.message),
            outcome.shots.len(),
            outcome.flags.len(),
            if position + 1 == results.len() {
                ""
            } else {
                ","
            }
        );
    }
    let _ = write!(
        summary,
        "  ],\n  \"failed\": {failed},\n  \"seconds\": {:.3},\n  \"report\": {}\n}}\n",
        elapsed.as_secs_f64(),
        json(&index.display().to_string())
    );
    let _ = fs::write(run_dir.join("summary.json"), summary);
    let mut flags = String::new();
    for (scenario, outcome) in results {
        for flag in &outcome.flags {
            let _ = writeln!(
                flags,
                "{}\t{}\t{}\t{}",
                scenario.name, flag.shot, flag.kind, flag.detail
            );
        }
    }
    let _ = fs::write(run_dir.join("review-flags.tsv"), flags);
    if let Some(parent) = run_dir.parent() {
        let _ = fs::write(parent.join("LATEST"), format!("{}\n", run_dir.display()));
    }
    index
}

pub fn print_summary(
    results: &[(&Scenario, Outcome)],
    elapsed: Duration,
    index: &Path,
    strict_review: bool,
) {
    println!("\n==== Acceptance summary ====");
    for (scenario, outcome) in results {
        println!(
            "{:<8} {:<28} {:>7.1} s  {}",
            outcome.status.label(),
            scenario.name,
            outcome.duration.as_secs_f64(),
            outcome.headline()
        );
        if outcome.status.is_failure(strict_review) && outcome.message.lines().count() > 1 {
            for line in outcome.message.lines().skip(1).take(12) {
                println!("{:<38}{line}", "");
            }
        }
    }
    let count = |wanted: Status| {
        results
            .iter()
            .filter(|(_, outcome)| outcome.status == wanted)
            .count()
    };
    let failed = results
        .iter()
        .filter(|(_, outcome)| outcome.status.is_failure(strict_review))
        .count();
    println!(
        "{} passed, {} warned, {} skipped, {failed} failed of {} in {:.1} s",
        count(Status::Pass),
        count(Status::Warn),
        count(Status::Skip),
        results.len(),
        elapsed.as_secs_f64()
    );
    println!("Report: {}", index.display());
    println!("Result: {}", if failed == 0 { "PASS" } else { "FAIL" });
}
