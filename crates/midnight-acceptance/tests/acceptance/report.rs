//! Results, screenshots and the HTML/JSON/text report of one run.
use super::{Options, Scenario, fixtures::Fixture};
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Finished runs kept in a run folder's parent, the new run included; older ones are
/// deleted when a run starts.
const KEPT_RUNS: usize = 5;
/// A run folder without its `summary.json` is a run still going (another checkout's, or
/// another package's in a shared folder) or one that was killed: it is deleted only once
/// it is older than this, which no run lasts.
const UNFINISHED_KEPT: Duration = Duration::from_secs(2 * 24 * 60 * 60);
/// Written into every run folder as it is created: pruning deletes only folders that
/// hold it (and, in the default `_build/acceptance`, run folders of earlier versions).
const RUN_MARKER: &str = ".acceptance-run";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Pass,
    /// Passed, but the UI review flagged layout problems.
    Warn,
    Fail,
    Skip,
    /// Every check passed except checks of announced behavior that is not implemented
    /// yet (each named, with its reason, in the message and notes). Not a failure.
    Pending,
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
            Self::Pending => "PENDING",
            Self::Timeout => "TIMEOUT",
            Self::NotRun => "NOT RUN",
        }
    }

    /// A scenario's `Err` message: `SKIP:` and `PENDING:` prefixes choose those
    /// statuses, anything else is a failure. Returns the status and the message without
    /// its prefix.
    pub fn of_error(message: &str) -> (Self, String) {
        for (prefix, status) in [("SKIP:", Self::Skip), ("PENDING:", Self::Pending)] {
            if let Some(rest) = message.strip_prefix(prefix) {
                return (status, rest.trim().to_owned());
            }
        }
        (Self::Fail, message.to_owned())
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
    stamp(seconds)
}

/// `YYYYMMDD-HHMMSS` in UTC of `seconds` since the Unix epoch.
fn stamp(seconds: u64) -> String {
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

/// A run folder's name: `YYYYMMDD-HHMMSS`, or `YYYYMMDD-HHMMSS-N` when a run that
/// started in the same second already took the plain name.
fn is_run_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let stamp = bytes.len() >= 15
        && bytes[8] == b'-'
        && bytes[..15]
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit());
    stamp
        && match bytes.get(15..) {
            Some([]) => true,
            Some([b'-', suffix @ ..]) => {
                !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit)
            }
            _ => false,
        }
}

/// A canonical path as a user spells it. Windows' `canonicalize` returns a verbatim
/// `\\?\C:\…` or `\\?\UNC\…` path, which the application would then show in its panels
/// and paths; a drive or UNC path drops that prefix (the standard library reaches long
/// paths without it).
fn plain(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(text) = path.to_str() {
        if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{share}"));
        }
        if let Some(drive) = text.strip_prefix(r"\\?\")
            && drive.as_bytes().get(1) == Some(&b':')
        {
            return PathBuf::from(drive);
        }
    }
    path
}

/// Create this run's own folder below `root`, which must be an absolute folder other
/// than a filesystem root: `root/<run id>`, made with `create_dir`, so it never is an
/// existing folder. Everything the run deletes later is inside it. Returns the
/// canonical path.
pub fn create_run(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("cannot create {}: {error}", root.display()))?;
    let root = plain(
        root.canonicalize()
            .map_err(|error| format!("cannot resolve {}: {error}", root.display()))?,
    );
    if root.parent().is_none() {
        return Err(format!(
            "{} is a filesystem root; name a folder for the run folders",
            root.display()
        ));
    }
    let run = super::fixtures::create_fresh(&root, &run_id())?;
    fs::write(run.join(RUN_MARKER), "")
        .map_err(|error| format!("cannot mark {} as a run: {error}", run.display()))?;
    Ok(run)
}

/// Prune the run folders beside `current`, the folder this run just created: keep the
/// newest [`KEPT_RUNS`] finished runs (with `summary.json`), `current` included, and
/// delete older finished runs and unfinished ones older than [`UNFINISHED_KEPT`]. Only
/// folders named like runs that hold the run marker are ever deleted; in the default
/// `_build/acceptance` (`default_root`) also run folders without one, which earlier
/// versions made. A run still going is never deleted, so runs in one folder never prune
/// each other's, and a run in its own `--run-dir` prunes only its own runs there.
pub fn prune_old_runs(current: &Path, default_root: bool) {
    let Some(parent) = current.parent() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let cutoff = stamp(now.saturating_sub(UNFINISHED_KEPT.as_secs()));
    let mut finished = Vec::new();
    for entry in fs::read_dir(parent).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if path == current
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
            || !is_run_name(&name)
            || !(default_root || path.join(RUN_MARKER).is_file())
        {
            continue;
        }
        if path.join("summary.json").is_file() {
            finished.push(path);
        } else if name[..15] < *cutoff {
            let _ = super::fixtures::remove_tree(&path);
        }
    }
    finished.sort();
    let excess = finished.len().saturating_sub(KEPT_RUNS.saturating_sub(1));
    for old in finished.into_iter().take(excess) {
        let _ = super::fixtures::remove_tree(&old);
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
.PASS{{color:var(--pass)}}.WARN,.PENDING{{color:var(--warn)}}.FAIL,.TIMEOUT,.NOTRUN{{color:var(--fail)}}.SKIP{{color:var(--muted)}}\
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
    update_latest(run_dir);
    index
}

/// Record `run_dir` in `LATEST` beside it. An existing `LATEST` is replaced only when
/// it is a pointer a run wrote: a small regular file holding one absolute path that
/// ends in a run folder's name (a run in Docker writes its container path). Anything
/// else there is the user's and stays as it is.
fn update_latest(run_dir: &Path) {
    let Some(root) = run_dir.parent() else {
        return;
    };
    let latest = root.join("LATEST");
    let replaceable = match fs::symlink_metadata(&latest) {
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        Ok(metadata) => {
            metadata.is_file()
                && metadata.len() <= 4096
                && fs::read_to_string(&latest).is_ok_and(|text| {
                    let line = text.strip_suffix('\n').unwrap_or(&text);
                    let named = Path::new(line);
                    !line.contains('\n')
                        && named.is_absolute()
                        && named
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(is_run_name)
                })
        }
    };
    if replaceable {
        let _ = fs::write(&latest, format!("{}\n", run_dir.display()));
    }
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
        if (outcome.status.is_failure(strict_review) || outcome.status == Status::Pending)
            && outcome.message.lines().count() > 1
        {
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
        "{} passed, {} warned, {} pending, {} skipped, {failed} failed of {} in {:.1} s",
        count(Status::Pass),
        count(Status::Warn),
        count(Status::Pending),
        count(Status::Skip),
        results.len(),
        elapsed.as_secs_f64()
    );
    println!("Report: {}", index.display());
    println!("Result: {}", if failed == 0 { "PASS" } else { "FAIL" });
}
