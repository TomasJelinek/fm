//! Archives through the installed native and archive filesystem plugins: Enter opens a
//! ZIP, a tar, a tar.gz and an empty ZIP; every listing equals the archive's members;
//! F5 copies a member out byte for byte; F3 views members; Enter on a ZIP inside a ZIP
//! opens it in the same panel (its listing, its path, F3 and F5 on its member, and
//! Backspace out through both levels); and a text file named `.zip` gives a clear
//! error instead of a listing or a hang.
//!
//! A second start of the application browses generated archives of every other
//! supported format: `.tgz`, `.tar.xz`, `.txz`, `.tar.bz2`, `.tbz2`, `.tbz`,
//! `.tar.zst`, `.tzst`, 7-Zip and a really deflated ZIP (each listing equals the
//! members, F3 shows a member, Space shows the next member, F5 copies one out byte for
//! byte), raw `.gz`, `.xz`, `.bz2` and `.zst` streams (one member with the payload),
//! three formats nested in each other (7-Zip ⊃ tar.gz ⊃ ZIP, entered and left level by
//! level), the four-level limit (Enter on a fifth level explains it), damaged
//! archives (truncated ZIP and 7-Zip, garbage after a 7-Zip signature, a tar.gz with a
//! corrupted stream: a clear error; a ZIP member with a wrong CRC: the listing works,
//! F3 and F5 of that member fail clearly), Compare files on two archive members (the
//! File A / File B form, as members have no native paths), and archives over the former
//! metadata limits of 100,000 entries and 8 MiB of names, and one folder of 100,001
//! files. Formats whose compressor this machine lacks are skipped with that reason.
use crate::{
    Context,
    app::{App, Config},
    encode, files_support as files, input, panel, viewers_stage,
    viewers_support::{self as support, Failures, clip, generated, millis},
};
use midnight_core::{CopyOperationState, PaneId};
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(40 * 60);
const LIST_LIMIT: Duration = Duration::from_secs(30);
/// Archives over the former metadata limits take longer to index.
const BIG_LIMIT: Duration = Duration::from_secs(180);

/// The members of `bundle.zip`, `source.tar` and `source.tar.gz` per folder.
const BUNDLE: &[(&str, &[&str])] = &[
    ("", &["bundle"]),
    (
        "bundle",
        &["empty", "nested", "readme.txt", "ünïcödé 🦀.txt"],
    ),
    ("bundle/nested", &["deeper", "notes.md"]),
    ("bundle/nested/deeper", &["data.bin"]),
    ("bundle/empty", &[]),
];

pub fn archives(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn shown(app: &App) -> BTreeSet<String> {
    app.names(PaneId::Left)
        .into_iter()
        .filter(|name| name != "..")
        .collect()
}

/// Wait for a finished listing of exactly `expected`; Err names what is shown.
fn wait_listing(app: &mut App, what: &str, expected: &[&str]) -> Result<Duration, String> {
    wait_listing_for(app, what, expected, LIST_LIMIT)
}

fn wait_listing_for(
    app: &mut App,
    what: &str,
    expected: &[&str],
    limit: Duration,
) -> Result<Duration, String> {
    let wanted: BTreeSet<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    let started = Instant::now();
    loop {
        app.settle();
        if !app.loading(PaneId::Left) && shown(app) == wanted {
            return Ok(started.elapsed());
        }
        if started.elapsed() >= limit {
            return Err(format!(
                "{what}: expected {wanted:?}, the panel shows {:?} (loading {}, error {:?}, \
                 notice {:?}, location {:?})",
                clip(&format!("{:?}", shown(app)), 300),
                app.loading(PaneId::Left),
                app.error(PaneId::Left),
                app.notice(),
                app.location(PaneId::Left)
            ));
        }
        app.idle(Duration::from_millis(20));
    }
}

/// Enter the focused-to-be row with Enter.
fn open_row(app: &mut App, name: &str) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, name)?;
    input::key(app.slint(), Key::Return);
    app.settle();
    Ok(())
}

/// Backspace until the panel shows `home` again with the cursor on `name`.
fn back_to(app: &mut App, home: &Path, levels: usize, name: &str) -> Result<(), String> {
    for _ in 0..levels {
        panel::wait_input(app);
        input::key(app.slint(), Key::Backspace);
        app.settle();
        let started = Instant::now();
        while app.loading(PaneId::Left) && started.elapsed() < LIST_LIMIT {
            app.idle(Duration::from_millis(20));
        }
    }
    let wanted = home.to_path_buf();
    let started = Instant::now();
    while app.location(PaneId::Left).as_ref() != Some(&wanted) || app.loading(PaneId::Left) {
        if started.elapsed() >= LIST_LIMIT {
            return Err(format!(
                "Backspace did not return to {}: location {:?}",
                home.display(),
                app.location(PaneId::Left)
            ));
        }
        app.idle(Duration::from_millis(20));
    }
    let names = app.names(PaneId::Left);
    let cursor = usize::try_from(app.cursor(PaneId::Left))
        .ok()
        .and_then(|row| names.get(row));
    if cursor.map(String::as_str) != Some(name) {
        return Err(format!(
            "back in {} the cursor is on {cursor:?}, not the archive left ({name})",
            home.display()
        ));
    }
    Ok(())
}

/// After a failed step: Backspace (at most eight times) until the panel shows `home`
/// again. Inside an archive the panel reports no native location.
fn recover(app: &mut App, home: &Path) -> Result<(), String> {
    let state = |app: &App| {
        format!(
            "location {:?}, shows {:?}, error {:?}, notice {:?}, loading {}, focused pane {}",
            app.location(PaneId::Left),
            clip(&format!("{:?}", shown(app)), 200),
            app.error(PaneId::Left),
            app.notice(),
            app.loading(PaneId::Left),
            app.window.get_focused_pane()
        )
    };
    let mut seen = Vec::new();
    for attempt in 1..=8 {
        let started = Instant::now();
        while app.loading(PaneId::Left) && started.elapsed() < LIST_LIMIT {
            app.idle(Duration::from_millis(20));
        }
        if app.location(PaneId::Left).as_deref() == Some(home) {
            return Ok(());
        }
        let before = state(app);
        if app.notice().is_some() {
            input::key(app.slint(), Key::Escape);
            app.settle();
        }
        // A click on the list keeps Backspace with the panel.
        panel::focus_list(app, PaneId::Left)?;
        input::key(app.slint(), Key::Backspace);
        app.idle(Duration::from_millis(300));
        let events = app.recent_events();
        seen.push(format!(
            "Backspace {attempt}: before: {before}; after: {}; last events: {}",
            state(app),
            events[events.len().saturating_sub(4)..].join(" / ")
        ));
    }
    let report = seen.join("\n");
    eprintln!("archive recovery diagnostics:\n{report}");
    Err(format!(
        "eight Backspaces did not return to {} (the panel is at {:?}):\n{report}",
        home.display(),
        app.location(PaneId::Left)
    ))
}

/// Walk the bundle tree inside `archive`, comparing each folder with its members.
fn walk_bundle(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    archive: &str,
) -> Result<(), String> {
    open_row(app, archive)?;
    let elapsed = wait_listing(app, &format!("{archive} root"), BUNDLE[0].1)?;
    context.note(format!(
        "Enter on {archive}: root listed in {}",
        millis(elapsed)
    ));
    context.note(format!(
        "latency\tarchive listing\t{archive}\t{}\troot",
        elapsed.as_millis()
    ));
    open_row(app, "bundle")?;
    wait_listing(app, &format!("{archive}/bundle"), BUNDLE[1].1)?;
    app.screenshot(
        context,
        &format!("{}-bundle", archive.replace('.', "-")),
        &format!("{archive}/bundle/ through the archive plugin"),
    );
    open_row(app, "nested")?;
    wait_listing(app, &format!("{archive}/bundle/nested"), BUNDLE[2].1)?;
    open_row(app, "deeper")?;
    wait_listing(app, &format!("{archive}/bundle/nested/deeper"), BUNDLE[3].1)?;
    // Back up to bundle/ (one listing at a time) and into the empty folder.
    input::key(app.slint(), Key::Backspace);
    wait_listing(app, &format!("{archive}/bundle/nested again"), BUNDLE[2].1)?;
    input::key(app.slint(), Key::Backspace);
    wait_listing(app, &format!("{archive}/bundle again"), BUNDLE[1].1)?;
    open_row(app, "empty")?;
    wait_listing(app, &format!("{archive}/bundle/empty"), BUNDLE[4].1)?;
    back_to(app, home, 3, archive)
}

fn run(context: &mut Context) -> Result<(), String> {
    let failures = Failures::default();
    fixture_archives(context, &failures)?;
    generated_archives(context, &failures)?;
    failures.finish(context)
}

fn fixture_archives(context: &mut Context, failures: &Failures) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let home = context.fixture.path("archives");
    let target = context.work.clone();
    let mut config = Config::new(&root, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    panel::enter(&mut app, PaneId::Left, "archives")?;

    for archive in ["bundle.zip", "source.tar", "source.tar.gz"] {
        failures.check(archive, walk_bundle(&mut app, context, &home, archive));
        recover(&mut app, &home)?;
    }

    failures.check("nested.zip", nested_zip(&mut app, context, &home, &target));
    recover(&mut app, &home)?;

    failures.check(
        "empty.zip",
        (|| {
            open_row(&mut app, "empty.zip")?;
            wait_listing(&mut app, "empty.zip", &[])?;
            app.screenshot(context, "empty-zip", "empty.zip: an empty listing");
            back_to(&mut app, &home, 1, "empty.zip")
        })(),
    );
    recover(&mut app, &home)?;

    failures.check(
        "copy out and view inside",
        copy_and_view(&mut app, context, &home, &target),
    );
    recover(&mut app, &home)?;

    failures.check(
        "not-a-zip.zip",
        refused(&mut app, context, &home, "not-a-zip.zip", "not-a-zip"),
    );
    panel::leave(&mut app, PaneId::Left)?;
    app.shutdown()
}

/// Enter on an archive the plugin cannot open: a clear error (panel or notice) within
/// the listing limit, never a listing or a hang, and the panel can return to `home`.
fn refused(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    archive: &str,
    shot: &str,
) -> Result<(), String> {
    open_row(app, archive)?;
    let started = Instant::now();
    loop {
        app.settle();
        let error = app.error(PaneId::Left);
        let notice = app.notice();
        if !error.is_empty() || notice.is_some() {
            app.screenshot(
                context,
                shot,
                &format!("Enter on {archive}: {error} {notice:?}"),
            );
            context.note(format!(
                "{archive}: panel error {error:?}, notice {notice:?}, location {:?}",
                app.location(PaneId::Left)
            ));
            let text = format!("{error} {notice:?}");
            if text.trim().len() < 12 {
                return Err(format!("{archive}: the error says too little: {text:?}"));
            }
            break;
        }
        if started.elapsed() >= LIST_LIMIT {
            app.screenshot(context, shot, &format!("Enter on {archive}: no error"));
            return Err(format!(
                "no error for {archive}; location {:?}, shows {:?}",
                app.location(PaneId::Left),
                shown(app)
            ));
        }
        app.idle(Duration::from_millis(20));
    }
    // The error must not leave the panel stuck: it shows `home` again or can return.
    if app.notice().is_some() {
        input::key(app.slint(), Key::Escape);
        app.settle();
    }
    recover(app, home)
}

fn copy_and_view(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    target: &Path,
) -> Result<(), String> {
    open_row(app, "bundle.zip")?;
    wait_listing(app, "bundle.zip root", BUNDLE[0].1)?;
    open_row(app, "bundle")?;
    wait_listing(app, "bundle.zip/bundle", BUNDLE[1].1)?;
    // F3 on a member.
    panel::select_row(app, PaneId::Left, "readme.txt")?;
    let viewing = support::view(app, false, Duration::from_secs(30))?;
    let shown = viewing.wait(
        app,
        "the member's text",
        Duration::from_secs(10),
        |window| window.get_accessible_text().contains("Inside the bundle."),
    );
    app.screenshot_window(
        context,
        &viewing.software,
        "view-member",
        &format!(
            "F3 on bundle.zip/bundle/readme.txt ({})",
            millis(viewing.latency)
        ),
    );
    context.note(format!(
        "latency\tarchive member\tbundle.zip/bundle/readme.txt\t{}\t{}",
        viewing.latency.as_millis(),
        clip(&viewing.status(), 60)
    ));
    let (status, latency) = (viewing.status(), viewing.latency);
    viewing.close(app)?;
    if let Err(error) = shown {
        // What the panel shows after the failed member view (finding V11).
        app.screenshot(
            context,
            "after-member-failure",
            &format!("The panel after the failed member view: {status}"),
        );
        context.note(format!(
            "After the failed member view: location {:?}, shows {:?}, error {:?}, notice {:?}, \
             focused pane {}; last events: {}",
            app.location(PaneId::Left),
            self::shown(app),
            app.error(PaneId::Left),
            app.notice(),
            app.window.get_focused_pane(),
            app.recent_events().join(" / ")
        ));
        return Err(format!(
            "F3 on an archive member failed after {}: {status} (finding V11); {error}",
            millis(latency)
        ));
    }
    copy_member(
        app,
        context,
        target,
        "readme.txt",
        b"Inside the bundle.\n",
        "copy-member",
    )?;
    context.note("F5 copied bundle.zip/bundle/readme.txt out byte for byte.");
    back_to(app, home, 2, "bundle.zip")
}

/// F5 on the archive member `name` copies it to the other panel, which shows `target`,
/// with exactly the bytes `expected`.
fn copy_member(
    app: &mut App,
    context: &mut Context,
    target: &Path,
    name: &str,
    expected: &[u8],
    shot: &str,
) -> Result<(), String> {
    let copied = target.join(name);
    let _ = std::fs::remove_file(&copied);
    panel::select_row(app, PaneId::Left, name)?;
    let start = app.copies.len();
    input::key(app.slint(), Key::F5);
    app.until("the Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    app.screenshot(
        context,
        shot,
        &format!("F5 on the archive member {name}: Copy dialog"),
    );
    input::key(app.slint(), Key::Return);
    app.until("the copy to finish", |app| {
        app.copies[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
    });
    let state = app.copies[start..]
        .iter()
        .find(|progress| progress.state.is_terminal())
        .map(|progress| progress.state);
    if state != Some(CopyOperationState::Completed) {
        return Err(format!("copying {name} ended {state:?}"));
    }
    let bytes = std::fs::read(&copied).map_err(|error| format!("{}: {error}", copied.display()))?;
    if bytes != expected {
        return Err(format!(
            "the copied {name} differs: {:?}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    Ok(())
}

/// Enter on a ZIP inside a ZIP opens it in the same panel: its listing, its path in
/// the path bar, F3 and F5 on its member, and Backspace back out through both levels,
/// each time with the cursor on the archive just left.
fn nested_zip(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    target: &Path,
) -> Result<(), String> {
    open_row(app, "nested.zip")?;
    wait_listing(app, "nested.zip root", &["inner.zip", "outer.txt"])?;
    // A member of the outer archive in the viewer.
    panel::select_row(app, PaneId::Left, "outer.txt")?;
    let viewing = support::view(app, false, Duration::from_secs(30))?;
    let outer = viewing.wait(app, "outer.txt", Duration::from_secs(10), |window| {
        window.get_accessible_text().contains("Outer.")
    });
    viewing.close(app)?;
    outer?;
    // Enter on the ZIP inside the ZIP lists its members, and the path bar names it.
    open_row(app, "inner.zip")?;
    let elapsed = wait_listing(app, "nested.zip/inner.zip", &["inner.txt"])?;
    let path = app.window.get_left_path().to_string();
    let separator = std::path::MAIN_SEPARATOR;
    app.screenshot(
        context,
        "nested-zip",
        &format!("inner.zip inside nested.zip, entered with Enter: {path}"),
    );
    if !path.ends_with(&format!("nested.zip{separator}inner.zip")) {
        return Err(format!(
            "inside nested.zip/inner.zip the path bar shows {path:?}"
        ));
    }
    context.note(format!(
        "Enter on nested.zip/inner.zip: listed in {} (path bar {path:?})",
        millis(elapsed)
    ));
    context.note(format!(
        "latency\tarchive listing\tnested.zip/inner.zip\t{}\troot",
        elapsed.as_millis()
    ));
    // A member of the inner archive in the viewer.
    view_member(app, context, "inner.txt", "Innermost.", None)?;
    // F5 copies the inner archive's member out byte for byte.
    copy_member(
        app,
        context,
        target,
        "inner.txt",
        b"Innermost.\n",
        "copy-nested-member",
    )?;
    context.note("F5 copied nested.zip/inner.zip/inner.txt out byte for byte.");
    // Backspace returns to nested.zip with the cursor on inner.zip, then to archives/.
    panel::wait_input(app);
    input::key(app.slint(), Key::Backspace);
    wait_listing(
        app,
        "nested.zip after leaving inner.zip",
        &["inner.zip", "outer.txt"],
    )?;
    cursor_on(app, "inner.zip", "back in nested.zip")?;
    back_to(app, home, 1, "nested.zip")
}

fn cursor_on(app: &App, name: &str, what: &str) -> Result<(), String> {
    let names = app.names(PaneId::Left);
    let cursor = usize::try_from(app.cursor(PaneId::Left))
        .ok()
        .and_then(|row| names.get(row));
    if cursor.map(String::as_str) != Some(name) {
        return Err(format!(
            "{what} the cursor is on {cursor:?}, not the archive left ({name})"
        ));
    }
    Ok(())
}

/// F3 on the member `name`: its text contains `text`; with `next`, Space shows the
/// next member, whose text contains that.
fn view_member(
    app: &mut App,
    context: &mut Context,
    name: &str,
    text: &str,
    next: Option<&str>,
) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, name)?;
    let viewing = support::view(app, false, Duration::from_secs(30))?;
    let shown = (|| {
        viewing.result.clone()?;
        viewing.wait(app, name, Duration::from_secs(15), |window| {
            window.get_accessible_text().contains(text)
        })?;
        context.note(format!(
            "latency\tarchive member\t{name}\t{}\t{}",
            viewing.latency.as_millis(),
            clip(&viewing.status(), 60)
        ));
        if let Some(next) = next {
            viewing.click_content(app);
            input::key(viewing.slint(), Key::Space);
            viewing.wait(
                app,
                "Space to show the next member",
                Duration::from_secs(15),
                |window| window.get_accessible_text().contains(next),
            )?;
        }
        Ok::<(), String>(())
    })();
    viewing.close(app)?;
    shown
}

// ---------------------------------------------------------------------------------------
// Generated archives of every format

/// What was generated, and the formats this machine could not make.
struct Formats {
    /// Archives holding the bundle tree.
    bundles: Vec<String>,
    /// Raw compressed streams of `note.txt`.
    streams: Vec<String>,
    /// The outermost archive of the three nested formats.
    nested: Option<String>,
    skipped: Vec<String>,
}

const NOTE: &[u8] = b"Compressed note.\n";

fn build_formats(folder: &Path, scratch: &Path) -> Result<Formats, String> {
    let mut formats = Formats {
        bundles: Vec::new(),
        streams: Vec::new(),
        nested: None,
        skipped: Vec::new(),
    };
    let members = generated::bundle_members();
    let tar = encode::tar(&members);
    for (name, bytes) in [
        ("bundle.tgz", Ok(encode::gzip(&tar))),
        ("bundle.tar.xz", generated::compress("xz", &tar, scratch)),
        ("bundle.txz", generated::compress("xz", &tar, scratch)),
        (
            "bundle.tar.bz2",
            generated::compress("bzip2", &tar, scratch),
        ),
        ("bundle.tbz2", generated::compress("bzip2", &tar, scratch)),
        ("bundle.tbz", generated::compress("bzip2", &tar, scratch)),
        ("bundle.tar.zst", generated::compress("zstd", &tar, scratch)),
        ("bundle.tzst", generated::compress("zstd", &tar, scratch)),
    ] {
        match bytes {
            Ok(bytes) => {
                generated::write(folder, name, bytes)?;
                formats.bundles.push(name.to_owned());
            }
            Err(reason) => formats.skipped.push(format!("{name}: {reason}")),
        }
    }
    let tree = scratch.join("tree");
    generated::materialize(&tree, &members)?;
    match generated::archive_folder("7z", &tree, "bundle", &folder.join("bundle.7z")) {
        Ok(()) => formats.bundles.push("bundle.7z".into()),
        Err(reason) => formats.skipped.push(format!("bundle.7z: {reason}")),
    }
    match generated::archive_folder("zip", &tree, "bundle", &folder.join("bundle-deflated.zip")) {
        Ok(()) => formats.bundles.push("bundle-deflated.zip".into()),
        Err(reason) => formats
            .skipped
            .push(format!("bundle-deflated.zip: {reason}")),
    }
    for (name, bytes) in [
        ("note.txt.gz", Ok(encode::gzip(NOTE))),
        ("note.txt.xz", generated::compress("xz", NOTE, scratch)),
        ("note.txt.bz2", generated::compress("bzip2", NOTE, scratch)),
        ("note.txt.zst", generated::compress("zstd", NOTE, scratch)),
    ] {
        match bytes {
            Ok(bytes) => {
                generated::write(folder, name, bytes)?;
                formats.streams.push(name.to_owned());
            }
            Err(reason) => formats.skipped.push(format!("{name}: {reason}")),
        }
    }
    // 7-Zip ⊃ tar.gz ⊃ ZIP ⊃ core.txt.
    let deep = encode::zip(&[encode::Member::file("core.txt", "The core.\n")]);
    let inner = encode::gzip(&encode::tar(&[encode::Member::file("deep.zip", deep)]));
    let nest = scratch.join("nest");
    generated::write(&nest, "inner.tar.gz", inner)?;
    match generated::archive_folder("7z", &nest, "inner.tar.gz", &folder.join("outer.7z")) {
        Ok(()) => formats.nested = Some("outer.7z".into()),
        Err(reason) => formats.skipped.push(format!("outer.7z: {reason}")),
    }
    // Five levels of ZIP: l1.zip ⊃ l2.zip ⊃ … ⊃ l5.zip ⊃ x.txt.
    let mut level = encode::zip(&[encode::Member::file("x.txt", "Level five.\n")]);
    for depth in (1..=4).rev() {
        level = encode::zip(&[encode::Member::file(&format!("l{}.zip", depth + 1), level)]);
    }
    generated::write(folder, "l1.zip", level)?;
    // Damaged archives.
    let zip = encode::zip(&members);
    generated::write(folder, "truncated.zip", &zip[..zip.len() * 3 / 5])?;
    let mut crc = encode::zip(&[encode::Member::file("readme.txt", "Inside the bundle.\n")]);
    // The member's first data byte follows its 30-byte local header and name.
    crc[30 + "readme.txt".len()] ^= 0x20;
    generated::write(folder, "crc.zip", crc)?;
    let mut corrupt = encode::gzip(&tar);
    let middle = corrupt.len() / 2;
    corrupt[middle] ^= 0xFF;
    generated::write(folder, "corrupt.tar.gz", corrupt)?;
    let mut garbage = b"7z\xBC\xAF\x27\x1C".to_vec();
    let mut random = generated::Random::new(0x5eed_0005);
    garbage.extend((0..4096).map(|_| random.next() as u8));
    generated::write(folder, "garbage.7z", garbage)?;
    if let Ok(seven) = std::fs::read(folder.join("bundle.7z")) {
        generated::write(folder, "truncated.7z", &seven[..seven.len() / 2])?;
    }
    // Over the former metadata limits: 100 folders of 1,001 empty files, and 90 folders
    // of 1,000 files with 99-byte names (8.9 MB of names).
    let mut many = Vec::with_capacity(100_200);
    for folder_index in 0..100 {
        many.push(encode::Member::directory(&format!("f{folder_index:03}")));
        for file in 0..1001 {
            many.push(encode::Member::file(
                &format!("f{folder_index:03}/{file:05}.txt"),
                Vec::new(),
            ));
        }
    }
    generated::write(folder, "many.tar", encode::tar(&many))?;
    let mut long = Vec::with_capacity(90_100);
    for folder_index in 0..90 {
        long.push(encode::Member::directory(&format!("n{folder_index:02}")));
        for file in 0..1000 {
            let stem = format!("n{folder_index:02}/{file:04}-");
            let name = format!("{stem}{}.txt", "x".repeat(99 - stem.len() - 4));
            long.push(encode::Member::file(&name, Vec::new()));
        }
    }
    generated::write(folder, "long-names.tar", encode::tar(&long))?;
    let mut flat = Vec::with_capacity(100_002);
    flat.push(encode::Member::directory("flat"));
    for file in 0..100_001 {
        flat.push(encode::Member::file(
            &format!("flat/{file:06}.txt"),
            flat_bytes(file),
        ));
    }
    generated::write(folder, "flat.tar", encode::tar(&flat))?;
    Ok(formats)
}

fn generated_archives(context: &mut Context, failures: &Failures) -> Result<(), String> {
    let home = context.work.join("formats");
    let target = context.work.join("copies");
    let scratch = context.work.join("scratch");
    for folder in [&home, &target, &scratch] {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let formats = build_formats(&home, &scratch)?;
    for skipped in &formats.skipped {
        failures.skip(&format!("archive {skipped}"), "no such compressor here");
    }
    let mut config = Config::new(&home, &target);
    config.plugins = Some(context.work.join("plugins"));
    // Folder sizes in exact bytes.
    config.size_bytes = true;
    let mut app = App::start(config);
    for archive in &formats.bundles {
        let result = walk_bundle(&mut app, context, &home, archive).and_then(|()| {
            open_row(&mut app, archive)?;
            wait_listing(&mut app, &format!("{archive} root"), BUNDLE[0].1)?;
            open_row(&mut app, "bundle")?;
            wait_listing(&mut app, &format!("{archive}/bundle"), BUNDLE[1].1)?;
            view_member(
                &mut app,
                context,
                "readme.txt",
                "Inside the bundle.",
                Some("Unicode member."),
            )?;
            copy_member(
                &mut app,
                context,
                &target,
                "readme.txt",
                b"Inside the bundle.\n",
                &format!("copy-{}", archive.replace('.', "-")),
            )?;
            back_to(&mut app, &home, 2, archive)
        });
        failures.check(archive, result);
        recover(&mut app, &home)?;
    }
    for stream in &formats.streams {
        failures.check(stream, raw_stream(&mut app, context, &home, stream));
        recover(&mut app, &home)?;
    }
    match &formats.nested {
        Some(outer) => failures.check(
            "7-Zip ⊃ tar.gz ⊃ ZIP",
            three_levels(&mut app, context, &home, &target, outer),
        ),
        None => failures.skip("7-Zip ⊃ tar.gz ⊃ ZIP", "no 7-Zip program here"),
    }
    recover(&mut app, &home)?;
    failures.check("four-level limit", depth_limit(&mut app, context, &home));
    recover(&mut app, &home)?;
    for (archive, shot) in [
        ("truncated.zip", "truncated-zip"),
        ("garbage.7z", "garbage-7z"),
        ("corrupt.tar.gz", "corrupt-tar-gz"),
    ] {
        failures.check(archive, refused(&mut app, context, &home, archive, shot));
    }
    if formats.bundles.iter().any(|name| name == "bundle.7z") {
        failures.check(
            "truncated.7z",
            refused(&mut app, context, &home, "truncated.7z", "truncated-7z"),
        );
    }
    failures.check("crc.zip", wrong_crc(&mut app, context, &home, &target));
    recover(&mut app, &home)?;
    failures.check(
        "Compare files on archive members",
        compare_members(&mut app, context, &home),
    );
    recover(&mut app, &home)?;
    failures.check(
        "many.tar (100,100 entries)",
        big(&mut app, context, &home, "many.tar", "f000", 100, 1001),
    );
    recover(&mut app, &home)?;
    failures.check(
        "long-names.tar (8.9 MB of names)",
        big(&mut app, context, &home, "long-names.tar", "n00", 90, 1000),
    );
    recover(&mut app, &home)?;
    failures.check(
        "flat.tar (one folder of 100,001 files)",
        big(&mut app, context, &home, "flat.tar", "flat", 1, 100_001),
    );
    recover(&mut app, &home)?;
    failures.check(
        "Ctrl+Shift+S on the folder of 100,001 files in flat.tar",
        flat_size(&mut app, context, &home),
    );
    recover(&mut app, &home)?;
    failures.check(
        "F5 of the folder of 100,001 files out of flat.tar",
        flat_copy(&mut app, context, &home, &target, false),
    );
    recover(&mut app, &home)?;
    failures.pending(
        "F5 of a selection of 100,001 members out of flat.tar",
        "copy-out of a selection of more than 4,096 entries is being enabled",
        flat_copy(&mut app, context, &home, &target, true),
    );
    files::dismiss_all(&mut app);
    recover(&mut app, &home)?;
    app.shutdown()
}

/// Member `index` of `flat.tar`'s folder: every thousandth file holds `index / 1000 + 1`
/// bytes, the others are empty (101 files, 5,151 bytes in all).
fn flat_bytes(index: usize) -> Vec<u8> {
    if index % 1000 == 0 {
        vec![b'f'; index / 1000 + 1]
    } else {
        Vec::new()
    }
}

/// The bytes all of `flat.tar`'s members hold.
const FLAT_TOTAL: u64 = 5_151;
/// F5 of 100,001 members must finish within this (debug build, SHA-256 readback).
const FLAT_COPY_BUDGET: Duration = Duration::from_secs(600);

/// Ctrl+Shift+S in flat.tar's root measures its folder: exactly [`FLAT_TOTAL`] bytes.
fn flat_size(app: &mut App, context: &mut Context, home: &Path) -> Result<(), String> {
    open_row(app, "flat.tar")?;
    wait_listing_for(app, "flat.tar root", &["flat"], BIG_LIMIT)?;
    panel::focus_list(app, PaneId::Left)?;
    let started = Instant::now();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "s");
    let measured = loop {
        app.settle();
        let row = app
            .rows(PaneId::Left)
            .into_iter()
            .find(|row| row.name == "flat");
        if let Some(row) = &row
            && row.size_state == 0
            && !row.size_running
            && row.size.chars().any(|character| character.is_ascii_digit())
        {
            break row.size.to_string();
        }
        if started.elapsed() >= BIG_LIMIT {
            return Err(format!(
                "Ctrl+Shift+S gave no size within {BIG_LIMIT:?}: {:?}",
                row.map(|row| (row.size.to_string(), row.size_state, row.size_running))
            ));
        }
        app.idle(Duration::from_millis(50));
    };
    let digits: String = measured.chars().filter(char::is_ascii_digit).collect();
    context.note(format!(
        "Ctrl+Shift+S on flat.tar/flat: {measured:?} in {}",
        millis(started.elapsed())
    ));
    app.screenshot(
        context,
        "flat-size",
        &format!("Ctrl+Shift+S in flat.tar: the folder of 100,001 files measures {measured}"),
    );
    if digits != FLAT_TOTAL.to_string() {
        return Err(format!(
            "the folder of 100,001 members measures {measured:?}, not {FLAT_TOTAL} bytes"
        ));
    }
    back_to(app, home, 1, "flat.tar")
}

/// F5 out of flat.tar: the folder `flat` itself, or (`selection`) all its 100,001
/// files marked with Ctrl+A. Every file arrives with its bytes, within the budget.
fn flat_copy(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    target: &Path,
    selection: bool,
) -> Result<(), String> {
    let arrived = if selection {
        target.to_path_buf()
    } else {
        target.join("flat")
    };
    let _ = std::fs::remove_dir_all(target.join("flat"));
    open_row(app, "flat.tar")?;
    wait_listing_for(app, "flat.tar root", &["flat"], BIG_LIMIT)?;
    if selection {
        open_row(app, "flat")?;
        let started = Instant::now();
        while shown(app).len() != 100_001 {
            if started.elapsed() >= BIG_LIMIT {
                return Err(format!("flat.tar/flat lists {} files", shown(app).len()));
            }
            app.idle(Duration::from_millis(50));
        }
        panel::focus_list(app, PaneId::Left)?;
        input::chord(app.slint(), &[Key::Control], "a");
        app.settle();
    } else {
        panel::select_row(app, PaneId::Left, "flat")?;
    }
    let start = app.copies.len();
    let started = Instant::now();
    input::key(app.slint(), Key::F5);
    // The Copy dialog, or a refusal.
    loop {
        app.settle();
        if !app.window.get_copy_draft_token().is_empty() {
            break;
        }
        if let Some((title, body)) = app.notice() {
            app.screenshot(
                context,
                "flat-copy-refused",
                &format!("F5: {title}: {body}"),
            );
            return Err(format!("F5 was refused: {title}: {body}"));
        }
        if started.elapsed() >= LIST_LIMIT {
            return Err("F5 opened neither the Copy dialog nor a notice".into());
        }
        app.idle(Duration::from_millis(20));
    }
    input::key(app.slint(), Key::Return);
    let copied = Instant::now();
    loop {
        app.settle();
        let ended = app.copies[start..]
            .iter()
            .find(|progress| progress.state.is_terminal())
            .map(|progress| progress.state);
        if let Some(state) = ended {
            if state != CopyOperationState::Completed {
                return Err(format!("the copy of 100,001 files ended {state:?}"));
            }
            break;
        }
        if let Some(window) = files::decision_window(app) {
            return Err(format!(
                "the copy asked a question: {}",
                files::describe_window(&window)
            ));
        }
        if copied.elapsed() >= FLAT_COPY_BUDGET {
            return Err(format!(
                "the copy of 100,001 files took longer than {FLAT_COPY_BUDGET:?}"
            ));
        }
        app.idle(Duration::from_millis(100));
    }
    let elapsed = copied.elapsed();
    context.note(format!(
        "latency\tarchive copy-out\t{} of flat.tar\t{}\t100,001 files",
        if selection { "selection" } else { "folder" },
        elapsed.as_millis()
    ));
    // Every file with its bytes.
    let mut problems = Vec::new();
    let mut count = 0;
    for index in 0..100_001 {
        let path = arrived.join(format!("{index:06}.txt"));
        match std::fs::read(&path) {
            Ok(bytes) => {
                count += 1;
                if bytes != flat_bytes(index) && problems.len() < 5 {
                    problems.push(format!("{} holds {} bytes", path.display(), bytes.len()));
                }
            }
            Err(error) if problems.len() < 5 => {
                problems.push(format!("{}: {error}", path.display()));
            }
            Err(_) => {}
        }
    }
    app.screenshot(
        context,
        if selection {
            "flat-copy-selection"
        } else {
            "flat-copy-folder"
        },
        &format!(
            "F5 out of flat.tar: {count} of 100,001 files in {}",
            millis(elapsed)
        ),
    );
    // The copies take inodes; this run's checks are done with them.
    if selection {
        for index in 0..100_001 {
            let _ = std::fs::remove_file(arrived.join(format!("{index:06}.txt")));
        }
    } else {
        let _ = std::fs::remove_dir_all(&arrived);
    }
    if count != 100_001 || !problems.is_empty() {
        return Err(format!(
            "{count} of 100,001 files arrived intact: {}",
            problems.join("; ")
        ));
    }
    back_to(app, home, if selection { 2 } else { 1 }, "flat.tar")
}

/// A raw compressed stream lists one member with the payload.
fn raw_stream(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    stream: &str,
) -> Result<(), String> {
    open_row(app, stream)?;
    let started = Instant::now();
    let member = loop {
        app.settle();
        let names = shown(app);
        if !app.loading(PaneId::Left) && names.len() == 1 {
            break names.into_iter().next().unwrap_or_default();
        }
        if started.elapsed() >= LIST_LIMIT {
            return Err(format!(
                "{stream} lists {:?} (error {:?}, notice {:?})",
                shown(app),
                app.error(PaneId::Left),
                app.notice()
            ));
        }
        app.idle(Duration::from_millis(20));
    };
    context.note(format!("{stream}: one member, {member:?}"));
    view_member(app, context, &member, "Compressed note.", None)?;
    back_to(app, home, 1, stream)
}

/// 7-Zip ⊃ tar.gz ⊃ ZIP ⊃ core.txt: each level entered with Enter (the path bar names
/// the chain), the core viewed and copied, then Backspace out level by level with the
/// cursor on each archive left.
fn three_levels(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    target: &Path,
    outer: &str,
) -> Result<(), String> {
    open_row(app, outer)?;
    wait_listing(app, outer, &["inner.tar.gz"])?;
    open_row(app, "inner.tar.gz")?;
    wait_listing(app, "inner.tar.gz", &["deep.zip"])?;
    open_row(app, "deep.zip")?;
    wait_listing(app, "deep.zip", &["core.txt"])?;
    let path = app.window.get_left_path().to_string();
    let separator = std::path::MAIN_SEPARATOR;
    let chain = format!("{outer}{separator}inner.tar.gz{separator}deep.zip");
    app.screenshot(
        context,
        "three-levels",
        &format!("deep.zip in inner.tar.gz in {outer}: {path}"),
    );
    if !path.ends_with(&chain) {
        return Err(format!("three levels deep the path bar shows {path:?}"));
    }
    view_member(app, context, "core.txt", "The core.", None)?;
    copy_member(
        app,
        context,
        target,
        "core.txt",
        b"The core.\n",
        "copy-core",
    )?;
    for (left, listing) in [
        ("deep.zip", &["deep.zip"][..]),
        ("inner.tar.gz", &["inner.tar.gz"][..]),
    ] {
        panel::wait_input(app);
        input::key(app.slint(), Key::Backspace);
        wait_listing(app, &format!("after leaving {left}"), listing)?;
        cursor_on(app, left, &format!("after leaving {left}"))?;
    }
    back_to(app, home, 1, outer)
}

/// Four levels open; Enter on a fifth explains the limit and the panel stays.
fn depth_limit(app: &mut App, context: &mut Context, home: &Path) -> Result<(), String> {
    open_row(app, "l1.zip")?;
    for depth in 2..=4 {
        let name = format!("l{depth}.zip");
        wait_listing(app, &format!("level {}", depth - 1), &[name.as_str()])?;
        open_row(app, &name)?;
    }
    wait_listing(app, "level 4", &["l5.zip"])?;
    let location = app.window.get_left_path().to_string();
    let notices = app.notice();
    open_row(app, "l5.zip")?;
    let started = Instant::now();
    let explained = loop {
        app.settle();
        let error = app.error(PaneId::Left);
        if let Some((title, body)) = app
            .notice()
            .filter(|notice| Some(notice) != notices.as_ref())
        {
            break format!("{title}: {body}");
        }
        if !error.is_empty() {
            break error;
        }
        if started.elapsed() >= LIST_LIMIT {
            return Err(format!(
                "Enter on a fifth archive level explained nothing; the panel shows {:?} at {:?}",
                shown(app),
                app.window.get_left_path()
            ));
        }
        app.idle(Duration::from_millis(20));
    };
    app.screenshot(
        context,
        "fifth-level",
        &format!("Enter on l5.zip at the fourth level: {explained}"),
    );
    context.note(format!("Fifth archive level: {explained:?}"));
    let lower = explained.to_lowercase();
    if !(lower.contains("level") || lower.contains("deep")) {
        return Err(format!(
            "the refusal does not explain the level limit: {explained:?}"
        ));
    }
    if app.notice().is_some() {
        input::key(app.slint(), Key::Escape);
        app.settle();
    }
    if app.window.get_left_path().as_str() != location {
        return Err(format!(
            "after the refusal the panel moved from {location:?} to {:?}",
            app.window.get_left_path()
        ));
    }
    recover(app, home)
}

/// A member with a wrong CRC: the archive lists, F3 and F5 of the member fail clearly.
fn wrong_crc(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    target: &Path,
) -> Result<(), String> {
    open_row(app, "crc.zip")?;
    wait_listing(app, "crc.zip", &["readme.txt"])?;
    panel::select_row(app, PaneId::Left, "readme.txt")?;
    let viewing = support::view(app, false, Duration::from_secs(30))?;
    let view = match &viewing.result {
        Err(error) => Ok(error.clone()),
        Ok(()) => {
            let shown = viewing.wait(app, "the CRC error", Duration::from_secs(10), |window| {
                window.get_status().to_lowercase().contains("crc")
                    || window.get_status().to_lowercase().contains("damaged")
            });
            shown.map(|()| viewing.status())
        }
    };
    app.screenshot_window(
        context,
        &viewing.software,
        "crc-member",
        "F3 on a ZIP member with a wrong CRC",
    );
    viewing.close(app)?;
    let view =
        view.map_err(|error| format!("F3 on a member with a wrong CRC showed no error: {error}"))?;
    if view.contains("Inside the bundle.") {
        return Err("F3 showed the corrupted member as if it were intact".into());
    }
    context.note(format!("crc.zip: F3 says {view:?}"));
    // F5 must not produce a file with the wrong bytes.
    let copied = target.join("readme.txt");
    let _ = std::fs::remove_file(&copied);
    let start = app.copies.len();
    panel::select_row(app, PaneId::Left, "readme.txt")?;
    input::key(app.slint(), Key::F5);
    app.until("the Copy dialog", |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    input::key(app.slint(), Key::Return);
    // The failed read either ends the job or asks what to do: Cancel job.
    let terminal = |app: &App| {
        app.copies[start..]
            .iter()
            .find(|progress| progress.state.is_terminal())
            .map(|progress| progress.state)
    };
    let started = Instant::now();
    let asked = loop {
        app.settle();
        if files::decision_window(app).is_some() {
            let window = files::wait_decision(app, "the failed copy's question");
            app.screenshot(context, "crc-copy", "F5 of a ZIP member with a wrong CRC");
            let text = files::describe_window(&window);
            files::answer(app, &window, &files::Choice::CancelJob)?;
            break Some(text);
        }
        if terminal(app).is_some() {
            app.screenshot(context, "crc-copy", "F5 of a ZIP member with a wrong CRC");
            break None;
        }
        if started.elapsed() >= LIST_LIMIT {
            return Err("F5 of a member with a wrong CRC neither ended nor asked".into());
        }
        app.idle(Duration::from_millis(20));
    };
    app.until("the failed copy to end", |app| {
        app.copies[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
    });
    let state = terminal(app);
    context.note(format!("crc.zip: F5 ended {state:?}; question: {asked:?}"));
    if state == Some(CopyOperationState::Completed) {
        return Err(format!(
            "F5 of a member with a wrong CRC completed ({} bytes written)",
            std::fs::metadata(&copied).map_or(0, |metadata| metadata.len())
        ));
    }
    if copied.exists() {
        return Err(format!("the failed copy left {} behind", copied.display()));
    }
    files::dismiss_all(app);
    back_to(app, home, 1, "crc.zip")
}

/// Ctrl+Shift+C on two marked archive members: members have no native paths, so the
/// comparator asks for File A and File B with empty fields.
fn compare_members(app: &mut App, context: &mut Context, home: &Path) -> Result<(), String> {
    open_row(app, "bundle.tgz")?;
    wait_listing(app, "bundle.tgz root", BUNDLE[0].1)?;
    open_row(app, "bundle")?;
    wait_listing(app, "bundle.tgz/bundle", BUNDLE[1].1)?;
    for name in ["readme.txt", "ünïcödé 🦀.txt"] {
        panel::select_row(app, PaneId::Left, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    let before = support::plugins(app)?.compared.borrow().len();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let compared = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .cloned();
    let result = match compared.and_then(|compared| compared.window.upgrade()) {
        None => Err(format!(
            "Ctrl+Shift+C on two archive members opened no comparator; notice {:?}",
            app.notice()
        )),
        Some(window) => {
            window
                .window()
                .dispatch_event(WindowEvent::WindowActiveChanged(true));
            app.idle(Duration::from_millis(300));
            support::paint_tool_windows(app);
            let checked = if !window.get_choosing_files() {
                Err(format!(
                    "the comparator compared archive members directly: {:?}",
                    window.get_status()
                ))
            } else if !window.get_file_a().is_empty() || !window.get_file_b().is_empty() {
                Err(format!(
                    "the File A / File B form shows {:?} and {:?} for archive members",
                    window.get_file_a(),
                    window.get_file_b()
                ))
            } else {
                Ok(())
            };
            if let Some(software) = support::software_window(app, window.window()) {
                app.screenshot_window(
                    context,
                    &software,
                    "compare-members",
                    "Ctrl+Shift+C on two archive members: File A and File B are asked for",
                );
            }
            input::key(window.window(), Key::Escape);
            app.idle(Duration::from_millis(200));
            app.slint()
                .dispatch_event(WindowEvent::WindowActiveChanged(true));
            app.settle();
            if window.window().is_visible() {
                return Err("Escape did not close the File A / File B form".into());
            }
            checked
        }
    };
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    back_to(app, home, 2, "bundle.tgz")?;
    result
}

/// An archive over the former metadata limits lists its `folders` folders and the
/// `files` files of the first one.
fn big(
    app: &mut App,
    context: &mut Context,
    home: &Path,
    archive: &str,
    first: &str,
    folders: usize,
    files: usize,
) -> Result<(), String> {
    open_row(app, archive)?;
    let started = Instant::now();
    loop {
        app.settle();
        let shown = shown(app);
        if !app.loading(PaneId::Left) && shown.len() == folders && shown.contains(first) {
            break;
        }
        let error = app.error(PaneId::Left);
        if !error.is_empty() || app.notice().is_some() || started.elapsed() >= BIG_LIMIT {
            let text = format!(
                "{archive} lists {} entries (error {error:?}, notice {:?}) after {}",
                shown.len(),
                app.notice(),
                millis(started.elapsed())
            );
            app.screenshot(context, &archive.replace('.', "-"), &text);
            if app.notice().is_some() {
                input::key(app.slint(), Key::Escape);
                app.settle();
            }
            return Err(text);
        }
        app.idle(Duration::from_millis(50));
    }
    context.note(format!(
        "latency\tarchive listing\t{archive}\t{}\troot",
        started.elapsed().as_millis()
    ));
    open_row(app, first)?;
    let started = Instant::now();
    loop {
        app.settle();
        if !app.loading(PaneId::Left) && shown(app).len() == files {
            break;
        }
        if started.elapsed() >= BIG_LIMIT {
            return Err(format!(
                "{archive}/{first} lists {} of {files} files",
                shown(app).len()
            ));
        }
        app.idle(Duration::from_millis(50));
    }
    back_to(app, home, 2, archive)
}
