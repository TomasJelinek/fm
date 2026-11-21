//! WIN-1 of the Windows acceptance run: with Details open, a Copy paused in the middle
//! of a large file shows the current file's progress ("Current file written: … / …
//! bytes" and its bar) without scrolling, in a window of its preferred size. The
//! Details panel takes a good part of the window's height, so that line must not wait
//! below the speed chart. Blue Desktop and Dark Touch with their default and larger
//! fonts (`minimum_size`), the 32 px touch font of `tool_reflow`, and the 760×560
//! window of the acceptance run. The row comes from the presenter, so the line is the
//! one the window really shows.
use super::polish;
use super::*;
use i_slint_core::{
    item_tree::{ItemRc, ItemTreeRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    layout::Orientation,
    window::WindowInner,
};
use midnight_ui::{JobProgressWindow, OperationRow};

/// The acceptance run's 1 GiB file and the bytes it had copied when it paused.
const LARGE: u64 = 1 << 30;
const COPIED: u64 = 257_687_552;

/// An appearance: its name, theme, density and font size.
type Appearance = (&'static str, i32, i32, i32);

const APPEARANCES: [Appearance; 6] = [
    ("Blue Desktop", 0, 0, 12),
    ("Blue Desktop", 0, 0, 14),
    ("Blue Desktop", 0, 0, 16),
    ("Dark Touch", 1, 1, 15),
    ("Dark Touch", 1, 1, 18),
    ("Blue Touch", 0, 1, 32),
];

/// A Copy paused in the middle of the large file, as the backend reports it.
fn paused_in_a_large_file(id: u64) -> CopyProgress {
    let mut progress = CopyProgress::queued(
        OperationId(id),
        "destination".into(),
        1,
        TransferIntent::Copy,
    );
    progress.state = CopyOperationState::Running;
    progress.phase = CopyOperationPhase::Copying;
    progress.paused = true;
    progress.pause_requested = true;
    progress.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::Recursive,
        files: 1,
        bytes: Some(LARGE),
        ..OperationDiscovery::default()
    };
    progress.current_path = Some("source/a folder with a rather long name/資料/large.bin".into());
    progress.current_file_size = Some(LARGE);
    progress.current_file_copied = COPIED;
    progress.copied_bytes = COPIED;
    progress.verification_pass_id = 1;
    progress
}

/// The window's logical size when the platform opens it.
fn preferred_size(window: &slint::Window) -> (u32, u32) {
    let component = WindowInner::from_pub(window).component();
    let component = ItemTreeRc::borrow_pin(&component);
    let size = |orientation: Orientation| {
        component
            .as_ref()
            .layout_info(orientation)
            .preferred_bounded()
            .ceil() as u32
    };
    (size(Orientation::Horizontal), size(Orientation::Vertical))
}

/// The scrolling area that shows `item`.
fn viewport(item: &ItemRc) -> Option<ItemRc> {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(current) = parent {
        if current.downcast::<Flickable>().is_some() {
            return Some(current);
        }
        parent = current.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

/// What keeps the current file's line or bar from being shown whole without scrolling.
fn hidden_file_progress(window: &MinimalSoftwareWindow, job: &OperationRow) -> Vec<String> {
    let line = modeless::accessible_item(window, AccessibleRole::Text, &job.file_progress_label);
    let bar = modeless::accessible_item(
        window,
        AccessibleRole::ProgressIndicator,
        &job.file_progress_title,
    );
    let mut findings = Vec::new();
    for (what, item) in [("the line", line), ("its bar", bar)] {
        let Some(item) = item else {
            findings.push(format!("{what} is missing"));
            continue;
        };
        let Some(area) = viewport(&item) else {
            findings.push(format!("{what} is outside the progress area"));
            continue;
        };
        let scrolled = area
            .downcast::<Flickable>()
            .map_or(0.0, |flickable| flickable.as_pin_ref().content_y().get());
        let (x, y, width, height) = polish::bounds(&item);
        let shown = polish::bounds(&area);
        let inside = width > 0.0
            && height > 0.0
            && x >= shown.0 - 0.5
            && y >= shown.1 - 0.5
            && x + width <= shown.0 + shown.2 + 0.5
            && y + height <= shown.1 + shown.3 + 0.5;
        if !inside || scrolled != 0.0 {
            findings.push(format!(
                "{what} at {:?} is not inside the progress area's {shown:?} (scrolled {scrolled})",
                (x, y, width, height)
            ));
        }
    }
    findings
}

#[test]
fn details_show_a_paused_large_files_progress_without_scrolling() {
    let h = Harness::new();
    h.start(1);
    h.apply(Event::Copy(CopyEvent::Progress(Arc::new(
        paused_in_a_large_file(1),
    ))));
    let job = h
        .main
        .get_operations()
        .iter()
        .find(|row| row.identity == "copy-job:1")
        .expect("the Copy is listed");
    assert!(
        job.active && job.paused && job.kind == 0 && !job.verification_view,
        "a paused Copy"
    );
    assert!(job.file_determinate, "the current file's size is known");
    assert!(
        job.file_progress_label
            .starts_with("Current file written: "),
        "{:?}",
        job.file_progress_label
    );
    assert_eq!(job.file_progress_title, "Current file written");
    let reported = APPEARANCES.into_iter().map(|appearance| (appearance, None));
    let acceptance = (("Blue Desktop", 0, 0, 12), Some((760, 560)));
    let mut findings = Vec::new();
    for ((name, scheme, density, font), size) in reported.chain([acceptance]) {
        let window = JobProgressWindow::new().unwrap();
        window.set_job(job.clone());
        window.show().unwrap();
        let native = h.windows.borrow().last().cloned().expect("the job window");
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        modeless::layout(&native, 900, 700);
        let (width, height) = size.unwrap_or_else(|| preferred_size(native.window()));
        let frame = format!("details-file-progress-{scheme}-{density}-{font}");
        modeless::draw(&native, width, height, &frame);
        modeless::click(&native, "Details");
        modeless::draw(&native, width, height, &frame);
        let what = format!("{name} {font}px at {width}x{height}");
        assert!(
            modeless::accessible_item(&native, AccessibleRole::Groupbox, "Details")
                .is_some_and(|details| details.is_visible()),
            "{what}: Details are open below the progress"
        );
        findings.extend(
            hidden_file_progress(&native, &job)
                .into_iter()
                .map(|finding| format!("{what}: {finding}")),
        );
        window.hide().unwrap();
    }
    assert!(findings.is_empty(), "{findings:#?}");
}
