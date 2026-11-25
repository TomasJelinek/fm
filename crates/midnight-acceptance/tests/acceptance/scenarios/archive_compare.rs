//! Compare actual archive member bytes with the same-named file in the opposite
//! panel. Both the archive and the comparator use their normal installed services.
use crate::{
    Context,
    app::{App, Config},
    input, panel, viewers_stage,
    viewers_support::{self as support, generated},
};
use midnight_core::PaneId;
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{
    env,
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(5 * 60);
const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;

pub fn compare(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

pub(super) enum Expected<'a> {
    Different([&'a str; 2]),
    Identical(&'a str),
    // The private file's lines need not be copied into the test or its reports.
    SuppliedDifference(&'a str),
}

impl Expected<'_> {
    fn verdict(&self) -> i32 {
        if matches!(self, Self::Identical(_)) {
            1
        } else {
            3
        }
    }

    fn rendered(&self, window: &midnight_compare_ui::ComparatorWindow) -> bool {
        let text = window.get_accessible_text();
        let samples = match self {
            Self::Different(columns) => *columns,
            Self::Identical(sample) => [*sample; 2],
            Self::SuppliedDifference(name) => {
                return window.get_left_name().ends_with(*name)
                    && window.get_right_name().ends_with(*name)
                    && text
                        .lines()
                        .flat_map(|line| line.split('\t'))
                        .any(|column| matches!(column.split_whitespace().nth(1), Some("−" | "+")));
            }
        };
        samples.iter().enumerate().all(|(side, sample)| {
            text.lines()
                .filter_map(|line| line.split_once('\t'))
                .any(|(left, right)| [left, right][side].contains(*sample))
        })
    }
}

/// Compare through the menu shortcut. The frame's accessible text is emitted by
/// the text renderer, with a tab between its left and right columns.
pub(super) fn compare_text(
    app: &mut App,
    context: &mut Context,
    expected: Expected<'_>,
    shot: Option<(&str, &str)>,
) -> Result<(), String> {
    let before = support::plugins(app)?.compared.borrow().len();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let window = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .and_then(|opened| opened.window.upgrade())
        .ok_or_else(|| {
            format!(
                "Compare files opened no comparator; notice {:?}",
                app.notice()
            )
        })?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let mut sought_difference = false;
    let deadline = Instant::now() + Duration::from_secs(45);
    let result = loop {
        app.settle();
        support::paint_tool_windows(app);
        if window.get_choosing_files() {
            break Err("Compare files asked for paths although both inputs were available".into());
        }
        if window.get_identity_state() == expected.verdict()
            && !window.get_bytes_mode()
            && !window.get_image_mode()
            && window.get_surface().size().width > 0
        {
            if expected.rendered(&window) {
                break Ok(());
            }
            if matches!(expected, Expected::SuppliedDifference(_)) && !sought_difference {
                input::key(window.window(), "n");
                sought_difference = true;
            }
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "expected text verdict {} and its two rendered columns; got {} ({})",
                expected.verdict(),
                window.get_identity_state(),
                window.get_status()
            ));
        }
        app.idle(Duration::from_millis(15));
    };
    if let Some((name, caption)) = shot {
        let software = support::software_window(app, window.window())
            .ok_or("the comparator is not visible")?;
        app.screenshot_window(context, &software, name, caption);
    }
    input::key(window.window(), Key::Escape);
    app.until("Escape to close the comparator", |_| {
        !window.window().is_visible()
    });
    app.slint()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    app.settle();
    result
}

/// Start with an archive's containing directory on the left and its native
/// counterpart directory on the right, then enter each archive folder with Enter.
fn open_member(
    context: &Context,
    archive: &Path,
    member: &str,
    native: &Path,
) -> Result<App, String> {
    let mut config = Config::new(archive.parent().ok_or("archive has no parent")?, native);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let mut entry = archive
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid archive name")?;
    for next in member.split('/') {
        panel::select_row(&mut app, LEFT, entry)?;
        let last_path = app.window.get_left_path();
        input::key(app.slint(), Key::Return);
        app.until(
            "archive directory containing the next path component",
            |app| {
                app.window.get_left_path() != last_path
                    && !app.loading(LEFT)
                    && app.names(LEFT).iter().any(|name| name == next)
            },
        );
        entry = next;
    }
    Ok(app)
}

fn run(context: &mut Context) -> Result<(), String> {
    let archive = context.fixture.path("archives/source.tar.gz");
    let native = context.work.join("native");
    let unicode = "ünïcödé 🦀.txt";
    generated::write(&native, "readme.txt", "From the native directory.\n")?;
    generated::write(&native, unicode, "Unicode member name\n")?;
    let mut app = open_member(context, &archive, "bundle/readme.txt", &native)?;
    for (active, inactive, shot) in [(LEFT, RIGHT, "from-archive"), (RIGHT, LEFT, "from-native")] {
        // With no marks, the counterpart is resolved by name even though the
        // other pane's cursor is on a different regular file.
        panel::select_row(&mut app, inactive, unicode)?;
        panel::select_row(&mut app, active, "readme.txt")?;
        compare_text(
            &mut app,
            context,
            Expected::Different(["Inside the bundle.", "From the native directory."]),
            Some((
                shot,
                "Archive member and same-named native file with different contents",
            )),
        )?;
    }
    panel::select_row(&mut app, LEFT, unicode)?;
    compare_text(
        &mut app,
        context,
        Expected::Identical("Unicode member name"),
        Some((
            "identical",
            "Byte-identical Unicode-named archive member and native file",
        )),
    )?;
    for name in ["readme.txt", unicode] {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    compare_text(
        &mut app,
        context,
        Expected::Different(["Inside the bundle.", "Unicode member name"]),
        Some(("two-members", "Two marked files inside the same archive")),
    )?;
    app.shutdown()?;
    drop(app);
    context.note("Ctrl+Shift+C resolved the same-name counterpart in both directions despite an unrelated cursor. The comparison rendered the differing texts, verified a byte-identical Unicode-named pair, and showed two marked members of the archive in display order.");

    // Optional read-only smoke on an existing text pair known to differ. The
    // comparator is closed without recording its source text or screenshot.
    match (
        env::var_os("MIDNIGHT_ACCEPTANCE_COMPARE_ARCHIVE"),
        env::var("MIDNIGHT_ACCEPTANCE_COMPARE_MEMBER").ok(),
        env::var_os("MIDNIGHT_ACCEPTANCE_COMPARE_NATIVE_DIR"),
    ) {
        (None, None, None) => Ok(()),
        (Some(archive), Some(member), Some(native)) => {
            let mut app = open_member(context, Path::new(&archive), &member, Path::new(&native))?;
            let name = member.rsplit('/').next().ok_or("missing member name")?;
            for pane in [LEFT, RIGHT] {
                panel::select_row(&mut app, pane, name)?;
                compare_text(&mut app, context, Expected::SuppliedDifference(name), None)?;
            }
            context.note("The supplied archive member and local file opened as a text difference in both directions.");
            app.shutdown()
        }
        _ => Err("Supply all three archive comparison inputs: ARCHIVE, MEMBER and NATIVE_DIR (MIDNIGHT_ACCEPTANCE_COMPARE_ prefix).".into()),
    }
}
