//! Find's budgets reached through the window, each an explicit Incomplete result whose
//! details name the limit: folder depth (128 retained folder levels: the files of the
//! first 127 nested folders are found), content read per file (a file over 1 GiB is
//! not read, the others are), an identical-files group larger than the 10,000-result
//! limit (omitted whole while a later group is listed), and the 100,000 duplicate
//! candidates. The result-count limit of an ordinary search is `tools_find_results`';
//! the visited-entries limit is reached through archives in `find_archives`. The total
//! content budget (16 GiB actually read) and retained memory (64 MiB) are not reached
//! here: the window cannot lower them, and reaching them would take minutes of reads.
use crate::{
    Context,
    app::{App, Config},
    scenarios::find_support::{
        self as find, BASE, Findings, Query, all_rows, fill, mentions, open_find, relative, run_for,
    },
    tools_support::{self as tools, Tool},
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

const LONG: Duration = Duration::from_secs(240);

fn io(path: &Path, error: std::io::Error) -> String {
    format!("{}: {error}", path.display())
}

/// Search `query` (with a long limit) and return the status, the rows and the texts of
/// the details page.
fn searched(
    app: &mut App,
    window: &Tool,
    query: Query<'_>,
) -> Result<(String, Vec<find::Row>, Vec<String>), String> {
    fill(app, window, query)?;
    let text = run_for(app, window, LONG)?;
    let rows = all_rows(app, window)?;
    let details = find::details(app, window)?;
    Ok((text, rows, details))
}

fn expect_limit(
    what: &str,
    text: &str,
    details: &[String],
    limit: &str,
    more: &[&str],
) -> Result<(), String> {
    if !text.starts_with("Incomplete") {
        return Err(format!("{what} ended {text:?}"));
    }
    let wanted = format!("Reached limits: {limit}");
    for needle in std::iter::once(wanted.as_str()).chain(more.iter().copied()) {
        if !mentions(details, needle) {
            return Err(format!("{what}: the details lack {needle:?}: {details:?}"));
        }
    }
    Ok(())
}

pub fn budgets(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("budgets");
    fs::create_dir_all(&root).map_err(|error| io(&root, error))?;
    let mut app = App::start(Config::new(&root, &context.work));
    let mut findings = Findings::default();
    let mut notes = Vec::new();
    findings.check("folder depth", depth(&mut app, context, &root, &mut notes));
    find::reset(&mut app);
    findings.check(
        "content read per file",
        per_file(&mut app, &root, &mut notes),
    );
    find::reset(&mut app);
    findings.check(
        "a group over the result limit",
        big_group(&mut app, context, &root, &mut notes),
    );
    find::reset(&mut app);
    findings.check(
        "duplicate candidates",
        candidates(&mut app, &root, &mut notes),
    );
    find::reset(&mut app);
    app.shutdown()?;
    for note in notes {
        context.note(note);
    }
    findings.into_result()
}

/// A Find window searching only `folder` (below the panel's folder).
fn window_on(app: &mut App, root: &Path, folder: &Path) -> Result<Tool, String> {
    let window = open_find(app)?;
    find::add_root(app, &window, folder, false)?;
    find::tick_root(app, &window, root, false)?;
    Ok(window)
}

/// 130 nested folders, a file in each: the files of the first 127 are found.
fn depth(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let base = root.join("depth");
    let now = SystemTime::now();
    let mut folder = base.clone();
    for level in 1..=130 {
        folder = folder.join(format!("d{level:03}"));
        tools::write_at(&folder.join(format!("leaf-{level:03}.txt")), b"leaf\n", now)?;
    }
    let window = window_on(app, root, &base)?;
    let (text, rows, details) = searched(
        app,
        &window,
        Query {
            masks: "leaf-*.txt",
            ..BASE
        },
    )?;
    let found: BTreeSet<String> = rows.iter().map(|row| row.name.clone()).collect();
    let expected: BTreeSet<String> = (1..=127)
        .map(|level| format!("leaf-{level:03}.txt"))
        .collect();
    app.screenshot_window(context, &window, "depth", "128 folder levels: Incomplete");
    find::close(app, &window);
    expect_limit("130 nested folders", &text, &details, "folder depth", &[])?;
    if found != expected {
        let missing: Vec<_> = expected.difference(&found).collect();
        let extra: Vec<_> = found.difference(&expected).collect();
        return Err(format!(
            "130 nested folders: missing {missing:?}, unexpected {extra:?}"
        ));
    }
    notes.push(format!("Folder depth: {text}"));
    Ok(())
}

/// A sparse file one byte over 1 GiB is not read; the small file is.
fn per_file(app: &mut App, root: &Path, notes: &mut Vec<String>) -> Result<(), String> {
    let base = root.join("content");
    tools::write_at(
        &base.join("small.txt"),
        b"a needle here\n",
        SystemTime::now(),
    )?;
    tools::sparse(&base.join("big.txt"), (1 << 30) + 1)?;
    let window = window_on(app, root, &base)?;
    let started = Instant::now();
    let (text, rows, details) = searched(
        app,
        &window,
        Query {
            masks: "*.txt",
            content: Some("needle"),
            ..BASE
        },
    )?;
    let took = started.elapsed();
    find::close(app, &window);
    expect_limit(
        "a file over 1 GiB",
        &text,
        &details,
        "content read per file",
        &["1 size-limited files"],
    )?;
    if relative(&rows, &base) != find::set(&["small.txt"]) {
        return Err(format!(
            "a file over 1 GiB: found {:?}",
            relative(&rows, &base)
        ));
    }
    notes.push(format!(
        "Content read per file: {text} in {} ms (the 1 GiB + 1 B file is not read)",
        took.as_millis()
    ));
    Ok(())
}

/// 10,001 identical empty files form a group larger than the 10,000-result limit: it is
/// omitted whole, and the later pair of identical files is still listed.
fn big_group(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let base = root.join("group");
    fs::create_dir_all(&base).map_err(|error| io(&base, error))?;
    for index in 0..10_001 {
        let path = base.join(format!("empty-{index:05}"));
        fs::write(&path, b"").map_err(|error| io(&path, error))?;
    }
    for name in ["pair-a.dat", "pair-b.dat"] {
        let path = base.join(name);
        fs::write(&path, b"ten bytes!").map_err(|error| io(&path, error))?;
    }
    let window = window_on(app, root, &base)?;
    let started = Instant::now();
    let (text, rows, details) = searched(
        app,
        &window,
        Query {
            masks: "*",
            identical: true,
            ..BASE
        },
    )?;
    let took = started.elapsed();
    app.screenshot_window(
        context,
        &window,
        "group-over-limit",
        "A 10,001-file group omitted, the pair listed",
    );
    find::close(app, &window);
    expect_limit("a 10,001-file group", &text, &details, "result count", &[])?;
    let counts = (
        find::detail_count(&details, "Identical files:", " verified groups"),
        find::detail_count(&details, "Identical files:", " candidates"),
    );
    if counts != (Some(1), Some(10_003)) {
        return Err(format!(
            "a 10,001-file group: {counts:?} groups and candidates in {details:?}"
        ));
    }
    let listed: Vec<(Option<String>, String)> = rows
        .iter()
        .map(|row| (row.group.clone(), row.name.clone()))
        .collect();
    let group = Some("Group 1 · 2 files".to_owned());
    if listed
        != [
            (group.clone(), "pair-a.dat".to_owned()),
            (group, "pair-b.dat".to_owned()),
        ]
    {
        return Err(format!("a 10,001-file group: listed {listed:?} ({text})"));
    }
    notes.push(format!(
        "A group over the result limit: {text} in {:.1} s",
        took.as_secs_f64()
    ));
    Ok(())
}

/// 100,001 files of different sizes (sparse): the 100,000-candidate ceiling ends the
/// collection; no group can exist, so nothing is read.
fn candidates(app: &mut App, root: &Path, notes: &mut Vec<String>) -> Result<(), String> {
    let base: PathBuf = root.join("candidates");
    fs::create_dir_all(&base).map_err(|error| io(&base, error))?;
    let created = Instant::now();
    for index in 0..100_001_u64 {
        let path = base.join(format!("c{index:06}"));
        tools::sparse(&path, index + 1)?;
    }
    let created = created.elapsed();
    let window = window_on(app, root, &base)?;
    let started = Instant::now();
    let (text, rows, details) = searched(
        app,
        &window,
        Query {
            masks: "*",
            identical: true,
            ..BASE
        },
    )?;
    let took = started.elapsed();
    find::close(app, &window);
    expect_limit(
        "100,001 candidates",
        &text,
        &details,
        "duplicate candidates",
        &[],
    )?;
    let counts = (
        find::detail_count(&details, "Identical files:", " verified groups"),
        find::detail_count(&details, "Identical files:", " candidates"),
    );
    if counts != (Some(0), Some(100_000)) {
        return Err(format!(
            "100,001 candidates: {counts:?} groups and candidates in {details:?}"
        ));
    }
    if !rows.is_empty() {
        return Err(format!(
            "100,001 different files listed {} rows",
            rows.len()
        ));
    }
    notes.push(format!(
        "Duplicate candidates: {text} in {:.1} s (100,001 sparse files created in {:.1} s)",
        took.as_secs_f64(),
        created.as_secs_f64()
    ));
    Ok(())
}
