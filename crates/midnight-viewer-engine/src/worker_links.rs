//! Links in viewed documents. The pointer over a link shows where it leads and the pointer
//! cursor; a press and release on the same link without a drag follows it; Tab and
//! Shift+Tab move a focus ring through the shown page's links and Enter follows the focused
//! one. What a link does is the shared policy (`midnight_core::classify_link`):
//!
//! - `#fragment` scrolls to the heading with that GitHub slug: on the shown page when it
//!   starts the document, otherwise through a bounded, cancellable scan of the document's
//!   pages that shows its progress after `SCAN_PROGRESS`.
//! - A local file a viewer shows opens in this window, with Back and Forward through the
//!   documents opened from links (the window is told whether each has a document to go
//!   to) and Reload reading the shown one again; a folder opens, and any other file is
//!   revealed, in the active panel. The file system is looked at on a thread of its own.
//! - `http`, `https` and `mailto` go to the system's default handler.
//! - Anything else is refused, and the status line says why.
//!
//! Markdown pages carry their links. The HTML viewer's links arrive as browser events
//! (`worker_web.rs`) and take the same path. Inside an archive, only files a viewer shows
//! in the document's folder or below open (through the document's own resources).
use super::*;
use crate::capture::CaptureEntry;
use crate::link_slug::Slugger;
use midnight_core::{
    LinkAction, LinkTarget, LocalLink, LocalPath, LocalTargetKind, ViewerLinkInput,
    ViewerLinkRequest, classify_link, describe_link, local_link_action,
};
use midnight_plugin_host::{ContentResources, api::document};
use std::{
    io::Read,
    path::Path,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "worker_links_tests.rs"]
mod tests;

/// Documents kept for Back, and for Forward.
const HISTORY: usize = 64;
/// The heading scan reads at most this many pages of a document.
const SCAN_PAGES: u32 = 256;
/// One scanned page's bytes and lines, within what the plugin protocol carries in a
/// viewport request (at most `MAX_PAGE_BYTES` and 4096 lines).
const SCAN_PAGE_BYTES: u32 = 64 * 1024;
const SCAN_PAGE_LINES: u32 = 4096;
/// A heading scan taking longer than this shows its progress.
const SCAN_PROGRESS: Duration = Duration::from_millis(300);
/// A file no viewer claims opens as text only when its first bytes hold no NUL.
const SNIFF_BYTES: usize = 4096;

pub(super) struct Links {
    /// Captures the linked files on disk that open in this window.
    native: crate::NativeCapture,
    /// The current file order's capture, kept for Back and Forward.
    order: Option<CaptureEntry>,
    /// Whether the documents are files on disk, not archive members or list labels.
    on_disk: bool,
    /// The shown document's own resources: its folder inside an archive.
    resources: Option<Arc<dyn ContentResources>>,
    /// The link under the pointer.
    hovered: Option<usize>,
    /// The link with the keyboard focus, and the source offset it starts at, which keeps
    /// the focus on that link when another page of the document replaces this one.
    focused: Option<(usize, Option<u64>)>,
    /// The focus state the window was told last.
    told: Option<(u32, Option<u32>)>,
    /// A local target being looked at on its own thread.
    resolving: Option<Arc<Mutex<Option<LinkAction>>>>,
    back: Vec<Visit>,
    forward: Vec<Visit>,
    /// Whether Back and Forward had a document to go to when the window was told last.
    told_history: Option<(bool, bool)>,
    /// Where to go once the opened document's first page is shown, and whether it is.
    arrival: Option<Arrival>,
    arrived: bool,
    scan: Option<Scan>,
}

/// A document in the link history: its file order and where it was shown.
struct Visit {
    files: Arc<[PathBuf]>,
    current: usize,
    order: Option<CaptureEntry>,
    on_disk: bool,
    position: u64,
}

enum Arrival {
    Fragment(String),
    Position(u64),
}

struct Scan {
    request: u64,
    fragment: String,
    slugs: Slugger,
    pages: u32,
    started: Instant,
}

impl Links {
    /// `focused` is the order's first capture: a single-file order has no capture of its
    /// own, and Back needs one to show that file again.
    pub(super) fn new(
        native: crate::NativeCapture,
        order: Option<CaptureEntry>,
        focused: Option<midnight_plugin_host::CapturedFile>,
        on_disk: bool,
    ) -> Self {
        let order = order.or_else(|| {
            let focused = focused?;
            let entry: CaptureEntry = Arc::new(move |_, _| Ok(focused.clone()));
            Some(entry)
        });
        Self {
            native,
            order,
            on_disk,
            resources: None,
            hovered: None,
            focused: None,
            told: None,
            resolving: None,
            back: Vec::new(),
            forward: Vec::new(),
            told_history: None,
            arrival: None,
            arrived: false,
            scan: None,
        }
    }

    /// Whether `id` is the heading scan's page request.
    pub(super) fn scanning(&self, id: u64) -> bool {
        self.scan.as_ref().is_some_and(|scan| scan.request == id)
    }
}

impl Worker {
    /// The opened document's resources, which resolve its links inside an archive.
    pub(super) fn link_document(&mut self, file: &midnight_plugin_host::CapturedFile) {
        self.links.resources = file.resources();
    }

    /// The pointer moved to `point` of the shown text (page layout coordinates), or left
    /// it (None), or the button was released there. Every hover is answered.
    pub(super) fn link_pointer(
        &mut self,
        point: Option<[f32; 2]>,
        release: bool,
        output: &Mailbox,
    ) {
        let link = point.and_then(|point| self.link_at(point));
        if release {
            // A click: the press put a collapsed selection in this link, and no drag
            // extended it.
            let clicked = link.filter(|index| {
                self.selection.is_some_and(|(start, end)| {
                    start == end
                        && self
                            .link_range(*index)
                            .is_some_and(|range| range.start <= start && start <= range.end)
                })
            });
            if let Some(index) = clicked {
                self.follow_page_link(index, output);
            }
            return;
        }
        self.links.hovered = link;
        let status = link.and_then(|index| self.page_link_status(index));
        output.put(ViewerOutput::LinkHover(status));
    }

    /// Answers a hover that no shown text reads, while the layout handshake waits or a
    /// video, an image or an HTML page is shown, with no link: the window asks its next
    /// hover question only after an answer. True when `input` was such a hover.
    pub(super) fn hover_without_text(&self, input: &ViewerInput, output: &Mailbox) -> bool {
        let unread = matches!(
            input,
            ViewerInput::Raster {
                input: ViewerRasterInput::Hover { .. },
                ..
            }
        ) && (self.layout.waiting()
            || self.mode == VIEWER_MODE_VIDEO
            || self.web.is_some()
            || self.image.is_some());
        if unread {
            output.put(ViewerOutput::LinkHover(None));
        }
        unread
    }

    /// The shown page's link whose painted text holds `point`.
    fn link_at(&self, point: [f32; 2]) -> Option<usize> {
        let (page, shaped) = (self.page.as_ref()?, self.shaped.as_ref()?);
        let links = &page.links.links;
        if links.is_empty() {
            return None;
        }
        let index = shaped.hit_test(point[0], point[1]).ok()?;
        let first = links.partition_point(|link| link.range.end < index);
        let [x, y] = point.map(f64::from);
        links[first..]
            .iter()
            .take_while(|link| link.range.start <= index)
            .position(|link| {
                shaped
                    .selection_geometry(link.range.clone())
                    .is_ok_and(|rects| {
                        rects
                            .iter()
                            .any(|rect| x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1)
                    })
            })
            .map(|offset| first + offset)
    }

    fn link_range(&self, index: usize) -> Option<std::ops::Range<usize>> {
        Some(self.page.as_ref()?.links.links.get(index)?.range.clone())
    }

    /// The folder relative links resolve against: the shown document's, on disk.
    fn link_folder(&self) -> Option<PathBuf> {
        self.links
            .on_disk
            .then(|| {
                self.files
                    .get(self.current)?
                    .parent()
                    .map(Path::to_path_buf)
            })
            .flatten()
    }

    /// Where the shown page's link `index` leads, for the status line.
    fn page_link_status(&self, index: usize) -> Option<String> {
        let target = self.page.as_ref()?.links.links.get(index)?.target.clone();
        Some(describe_link(&target, self.link_folder().as_deref()))
    }

    /// The status line for a browser's hovered link target, or None when it left one.
    pub(super) fn web_link_status(&self, target: &str) -> Option<String> {
        (!target.is_empty()).then(|| describe_link(target, self.link_folder().as_deref()))
    }

    pub(super) fn link_input(&mut self, input: ViewerLinkInput, output: &Mailbox) {
        match input {
            ViewerLinkInput::Next => self.move_link_focus(true, output),
            ViewerLinkInput::Previous => self.move_link_focus(false, output),
            ViewerLinkInput::Follow => {
                if let Some((index, _)) = self.links.focused {
                    self.follow_page_link(index, output);
                }
            }
            ViewerLinkInput::Back => self.link_history(true, output),
            ViewerLinkInput::Forward => self.link_history(false, output),
            ViewerLinkInput::Reload => self.reload(output),
        }
    }

    /// Tell the window whether Back and Forward have a document to go to, when that
    /// changed.
    pub(super) fn tell_link_history(&mut self, output: &Mailbox) {
        let history = (!self.links.back.is_empty(), !self.links.forward.is_empty());
        if self.links.told_history != Some(history) {
            self.links.told_history = Some(history);
            output.put(ViewerOutput::LinkHistory {
                back: history.0,
                forward: history.1,
            });
        }
    }

    /// Read the shown document again from its file, where it was shown. The history stays;
    /// an HTML page keeps its Online and JavaScript choices.
    fn reload(&mut self, output: &Mailbox) {
        self.cancel_link_work();
        let visit = self.visit();
        self.web_choices.reload = self
            .web
            .as_ref()
            .zip(self.files.get(self.current))
            .map(|(web, path)| (path.clone(), web.policy()));
        self.replace_order(visit.files, visit.current, visit.order, visit.on_disk);
        self.links.arrival = Some(Arrival::Position(visit.position));
        self.open(output);
    }

    /// Tab moves the focus to the next link, the first one in view to begin with; Shift+Tab
    /// to the previous one, and from the first one off the links.
    fn move_link_focus(&mut self, forward: bool, output: &Mailbox) {
        let count = self.page.as_ref().map_or(0, |page| page.links.links.len());
        let next = match (self.links.focused.map(|(index, _)| index), forward) {
            _ if count == 0 => None,
            (None, true) => Some(self.first_link_in_view().unwrap_or(0)),
            (None, false) | (Some(0), false) => None,
            (Some(index), true) => Some((index + 1).min(count - 1)),
            (Some(index), false) => Some(index - 1),
        };
        self.links.focused = next.map(|index| (index, self.link_source(index)));
        match next {
            Some(index) => {
                self.reveal_link(index);
                if let Some(status) = self.page_link_status(index) {
                    self.status(status, output);
                }
            }
            None => self.status(String::new(), output),
        }
        self.raster.invalidate();
        self.dirty = true;
        self.tell_link_focus(output);
    }

    /// The first link whose text is below the top of the view.
    fn first_link_in_view(&self) -> Option<usize> {
        let (page, shaped) = (self.page.as_ref()?, self.shaped.as_ref()?);
        let top = f64::from(self.origin[1]);
        page.links.links.iter().position(|link| {
            shaped
                .selection_geometry(link.range.clone())
                .is_ok_and(|rects| rects.first().is_some_and(|rect| rect.y1 > top))
        })
    }

    /// The source offset at which link `index` starts, where an exact anchor names it.
    fn link_source(&self, index: usize) -> Option<u64> {
        let page = self.page.as_ref()?;
        let start = page.links.links.get(index)?.range.start;
        page.anchors
            .iter()
            .find(|anchor| anchor.text_offset as usize == start)
            .map(|anchor| anchor.source_offset)
    }

    /// Scroll link `index` into the view when it is not in it.
    fn reveal_link(&mut self, index: usize) {
        let Some(range) = self.link_range(index) else {
            return;
        };
        let Some(rects) = self
            .shaped
            .as_ref()
            .and_then(|shaped| shaped.selection_geometry(range).ok())
        else {
            return;
        };
        let Some(first) = rects.first() else {
            return;
        };
        let top = f64::from(self.origin[1]);
        let height = f64::from(self.geometry.height);
        if first.y0 >= top && first.y1 <= top + height {
            return;
        }
        self.origin[1] = ((first.y0 + first.y1 - height) / 2.0).max(0.0) as f32;
        let width = self.text_width();
        let (left, right) = (first.x0 as f32, first.x1 as f32);
        if left < self.origin[0] || right > self.origin[0] + width {
            self.origin[0] = ((left + right - width) / 2.0).max(0.0);
        }
        self.clamp_origin();
        self.reset_raster_context();
        self.frames.invalidate();
    }

    /// Tell the window the shown page's links and their focus, when that changed.
    fn tell_link_focus(&mut self, output: &Mailbox) {
        let count = self.page.as_ref().map_or(0, |page| {
            page.links.links.len().min(u32::MAX as usize) as u32
        });
        let focused = self
            .links
            .focused
            .and_then(|(index, _)| u32::try_from(index).ok());
        if self.links.told != Some((count, focused)) {
            self.links.told = Some((count, focused));
            output.put(ViewerOutput::LinkFocus { count, focused });
        }
    }

    /// The focused link's range, for the focus ring.
    pub(super) fn focused_link_range(&self) -> Option<std::ops::Range<usize>> {
        self.link_range(self.links.focused?.0)
    }

    /// Draw the focus ring into `pixels`, a `width` x `height` premultiplied RGBA raster
    /// whose pixel (0, 0) is page layout point `origin`.
    pub(super) fn paint_link_focus(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        origin: [f32; 2],
    ) {
        let Some(range) = self.focused_link_range() else {
            return;
        };
        let Some(rects) = self
            .shaped
            .as_ref()
            .and_then(|shaped| shaped.selection_geometry(range).ok())
        else {
            return;
        };
        let [red, green, blue, _] = self.appearance.accent;
        let ring = (2.0 * self.geometry.scale).round().max(1.0) as i64;
        let [x, y] = origin.map(f64::from);
        for rect in rects {
            let edges = [
                ((rect.x0 - x).floor() as i64).saturating_sub(ring),
                ((rect.y0 - y).floor() as i64).saturating_sub(ring),
                ((rect.x1 - x).ceil() as i64).saturating_add(ring),
                ((rect.y1 - y).ceil() as i64).saturating_add(ring),
            ];
            paint_ring(
                pixels,
                [width, height],
                edges,
                ring,
                [red, green, blue, 255],
            );
        }
    }

    /// A new page is shown: keep the focus on the same link where the page still holds it,
    /// and go where the opened document was asked to.
    pub(super) fn link_page_shown(&mut self, output: &Mailbox) {
        self.links.hovered = None;
        self.links.focused = self.links.focused.and_then(|(_, source)| {
            let source = source?;
            let page = self.page.as_ref()?;
            let index = page.links.links.iter().position(|link| {
                page.anchors.iter().any(|anchor| {
                    anchor.text_offset as usize == link.range.start
                        && anchor.source_offset == source
                })
            })?;
            Some((index, Some(source)))
        });
        self.tell_link_focus(output);
        // Requests wait for the loop's next pass, after this page's own request ended.
        self.links.arrived = self.links.arrival.is_some();
    }

    /// A document opened from a link, or from the history, goes where it was asked to.
    fn arrive(&mut self, output: &Mailbox) {
        if !std::mem::take(&mut self.links.arrived) {
            return;
        }
        match self.links.arrival.take() {
            Some(Arrival::Fragment(fragment)) => self.go_to_fragment(fragment, output),
            Some(Arrival::Position(position)) if position > 0 => {
                self.request_page(position, false, output)
            }
            _ => {}
        }
    }

    /// A document opens: nothing of the previous one's links stays.
    pub(super) fn links_reset(&mut self, output: &Mailbox) {
        self.cancel_link_work();
        self.links.hovered = None;
        self.links.focused = None;
        self.links.resources = None;
        self.links.arrived = false;
        self.tell_link_focus(output);
    }

    fn follow_page_link(&mut self, index: usize, output: &Mailbox) {
        let target = self
            .page
            .as_ref()
            .and_then(|page| page.links.links.get(index))
            .map(|link| link.target.clone());
        if let Some(target) = target {
            self.follow_link(&target, output);
        }
    }

    /// Follow a link to `target`, as the shown document states it.
    pub(super) fn follow_link(&mut self, target: &str, output: &Mailbox) {
        self.cancel_link_work();
        match classify_link(target) {
            LinkTarget::Fragment(fragment) => self.go_to_fragment(fragment, output),
            LinkTarget::External(url) => {
                let place = if url.is_mail() {
                    "your mail program"
                } else {
                    "your browser"
                };
                self.link_notice(format!("Opening in {place}…"), output);
                output.put(ViewerOutput::LinkRequest(ViewerLinkRequest::OpenExternal(
                    url,
                )));
            }
            LinkTarget::Refused(refusal) => self.link_notice(refusal.message(), output),
            LinkTarget::Local(link) => self.follow_local(link, output),
        }
    }

    /// A message about a link: on the status line of a Markdown page, or as the HTML
    /// viewer's notice, which stays until the next action.
    pub(super) fn link_notice(&mut self, message: String, output: &Mailbox) {
        if self.web.is_some() {
            self.web_link_notice(message, output);
        } else {
            self.status(message, output);
        }
    }

    fn follow_local(&mut self, link: LocalLink, output: &Mailbox) {
        if !self.links.on_disk {
            self.follow_archive_member(link, output);
            return;
        }
        let Some(folder) = self.link_folder() else {
            self.link_notice(
                "This document has no folder its links could lead from.".into(),
                output,
            );
            return;
        };
        let path = link.resolve(&folder);
        let by_name = self.viewer_claims(&path);
        let slot = Arc::new(Mutex::new(None));
        let (result, wake, fragment) = (slot.clone(), self.wake.clone(), link.fragment);
        let started = std::thread::Builder::new()
            .name("viewer-link".into())
            .spawn(move || {
                let kind = local_kind(&path, by_name);
                let action = local_link_action(path, fragment, kind);
                *result.lock().unwrap_or_else(PoisonError::into_inner) = Some(action);
                wake();
            });
        match started {
            Ok(_) => self.links.resolving = Some(slot),
            Err(error) => self.link_notice(format!("Cannot follow the link: {error}"), output),
        }
    }

    /// Whether a viewer claims `path` by its name: a plugin with its extension, an image,
    /// HTML or video provider, or the Office preview, not only the Basic viewer.
    fn viewer_claims(&self, path: &Path) -> bool {
        match self.route(path) {
            Some(crate::routing::Route::Native { index, .. }) => self
                .runtime
                .catalog()
                .get(index)
                .is_some_and(|info| !info.extensions.is_empty()),
            Some(_) => true,
            None => false,
        }
    }

    /// A finished look at a local target, and where an opened document goes.
    pub(super) fn poll_links(&mut self, output: &Mailbox) {
        self.arrive(output);
        let Some(action) = self
            .links
            .resolving
            .as_ref()
            .and_then(|slot| slot.lock().unwrap_or_else(PoisonError::into_inner).take())
        else {
            return;
        };
        self.links.resolving = None;
        match action {
            LinkAction::ScrollTo(fragment) => self.go_to_fragment(fragment, output),
            LinkAction::OpenInViewer { path, fragment } => {
                let capture = self.links.native.clone();
                let file = path.clone();
                let entry: CaptureEntry = Arc::new(move |_, closed| capture(&file, closed));
                self.open_linked(path, Some(entry), true, fragment, output);
            }
            LinkAction::OpenFolder(path) => {
                self.link_notice(
                    format!(
                        "Opening {} in the active panel.",
                        midnight_core::link_status_text(&path.to_string_lossy())
                    ),
                    output,
                );
                output.put(ViewerOutput::LinkRequest(ViewerLinkRequest::OpenFolder(
                    path,
                )));
            }
            LinkAction::Reveal(path) => {
                self.link_notice(
                    format!(
                        "No viewer shows {}; it is shown in the active panel.",
                        midnight_core::link_status_text(&path.to_string_lossy())
                    ),
                    output,
                );
                output.put(ViewerOutput::LinkRequest(ViewerLinkRequest::Reveal(path)));
            }
            LinkAction::OpenExternal(url) => output.put(ViewerOutput::LinkRequest(
                ViewerLinkRequest::OpenExternal(url),
            )),
            LinkAction::Refuse(message) => self.link_notice(message, output),
        }
    }

    /// Inside an archive, a file a viewer shows in the document's folder or below opens
    /// through the document's own resources. Nothing else can be reached from there yet.
    fn follow_archive_member(&mut self, link: LocalLink, output: &Mailbox) {
        let names = match link.path {
            LocalPath::Relative { up: 0, names } if !names.is_empty() => names,
            LocalPath::Relative { up: 0, .. } => {
                self.link_notice(
                    "Links to folders inside an archive are not followed yet.".into(),
                    output,
                );
                return;
            }
            LocalPath::Relative { .. } => {
                self.link_notice(
                    "Links that lead above the document's folder inside an archive are not \
                     followed yet."
                        .into(),
                    output,
                );
                return;
            }
            LocalPath::Rooted { .. } => {
                self.link_notice(
                    "Links to absolute paths are not followed from inside an archive.".into(),
                    output,
                );
                return;
            }
        };
        let Some(resources) = self.links.resources.clone() else {
            self.link_notice(
                "This document cannot reach other files, so its links to files are not \
                 followed."
                    .into(),
                output,
            );
            return;
        };
        let name = names.last().cloned().unwrap_or_default();
        if !self.viewer_claims(Path::new(&name)) {
            self.link_notice(
                format!(
                    "No viewer shows {}, and files inside an archive cannot be shown in the \
                     panel from here yet.",
                    midnight_core::link_status_text(&name)
                ),
                output,
            );
            return;
        }
        let entry: CaptureEntry = Arc::new(move |_, _| {
            resources
                .capture(&names)
                .map_err(|error| format!("Cannot open the linked file: {error}"))
        });
        self.open_linked(
            PathBuf::from(name),
            Some(entry),
            false,
            link.fragment,
            output,
        );
    }

    /// Open `path` in this window, after the shown document in the history.
    fn open_linked(
        &mut self,
        path: PathBuf,
        order: Option<CaptureEntry>,
        on_disk: bool,
        fragment: Option<String>,
        output: &Mailbox,
    ) {
        let visit = self.visit();
        push_bounded(&mut self.links.back, visit);
        self.links.forward.clear();
        self.tell_link_history(output);
        self.replace_order(Arc::from([path]), 0, order, on_disk);
        self.links.arrival = fragment.map(Arrival::Fragment);
        self.open(output);
    }

    fn visit(&self) -> Visit {
        Visit {
            files: self.files.clone(),
            current: self.current,
            order: self.links.order.clone(),
            on_disk: self.links.on_disk,
            position: self.page.as_ref().map_or(0, |page| page.start),
        }
    }

    fn replace_order(
        &mut self,
        files: Arc<[PathBuf]>,
        current: usize,
        order: Option<CaptureEntry>,
        on_disk: bool,
    ) {
        self.files = files;
        self.current = current;
        self.links.order = order.clone();
        self.links.on_disk = on_disk;
        self.captured = None;
        self.capture_request = None;
        self.prefetch = order.and_then(|capture| {
            crate::capture_prefetch::Prefetch::spawn(
                self.files.len(),
                current,
                None,
                capture,
                self.window_closed.clone(),
                self.wake.clone(),
            )
            .ok()
        });
    }

    /// Back to the document a link was followed from, or forward again. Without one in
    /// that direction nothing happens.
    fn link_history(&mut self, back: bool, output: &Mailbox) {
        let visit = if back {
            self.links.back.pop()
        } else {
            self.links.forward.pop()
        };
        let Some(visit) = visit else {
            return;
        };
        self.cancel_link_work();
        let current = self.visit();
        if back {
            push_bounded(&mut self.links.forward, current);
        } else {
            push_bounded(&mut self.links.back, current);
        }
        self.tell_link_history(output);
        self.replace_order(visit.files, visit.current, visit.order, visit.on_disk);
        self.links.arrival = Some(Arrival::Position(visit.position));
        self.open(output);
    }

    /// Scroll to the heading `fragment` names. An empty fragment is the top.
    fn go_to_fragment(&mut self, fragment: String, output: &Mailbox) {
        if fragment.is_empty() {
            self.request_page(0, false, output);
            return;
        }
        if self.mode != viewer::MODE_DOCUMENT {
            self.link_notice(
                format!(
                    "This view has no headings, so #{} is not shown.",
                    midnight_core::link_status_text(&fragment)
                ),
                output,
            );
            return;
        }
        let mut slugs = Slugger::default();
        let mut from = 0;
        if let Some(page) = &self.page
            && !page.before
        {
            let found = page.links.headings.iter().find_map(|heading| {
                let text = page.text.get(heading.range.clone())?;
                slugs.matches(text, &fragment).then_some(heading.source)
            });
            if let Some(source) = found {
                self.request_page(source, false, output);
                return;
            }
            if !page.after {
                self.fragment_missing(&fragment, output);
                return;
            }
            from = page.end;
        }
        self.request_scan(
            Scan {
                request: 0,
                fragment,
                slugs,
                pages: 0,
                started: Instant::now(),
            },
            from,
            output,
        );
    }

    fn fragment_missing(&mut self, fragment: &str, output: &Mailbox) {
        if fragment.eq_ignore_ascii_case("top") {
            self.request_page(0, false, output);
        } else {
            self.link_notice(
                format!(
                    "This document has no heading #{}.",
                    midnight_core::link_status_text(fragment)
                ),
                output,
            );
        }
    }

    /// Ask for the scan's next page, from source offset `from`.
    fn request_scan(&mut self, mut scan: Scan, from: u64, output: &Mailbox) {
        if scan.pages >= SCAN_PAGES {
            self.link_notice(
                format!(
                    "#{} was not found in the first {SCAN_PAGES} pages of this document.",
                    midnight_core::link_status_text(&scan.fragment)
                ),
                output,
            );
            return;
        }
        let viewport = messages::Viewport {
            source_anchor: from,
            backwards: false,
            max_bytes: SCAN_PAGE_BYTES,
            max_lines: SCAN_PAGE_LINES,
        };
        let requested = match &self.session {
            Some(session) => session
                .viewport(viewport)
                .map_err(|error| error.to_string()),
            // A simulated provider admits the request as a session would.
            #[cfg(test)]
            None => {
                super::unsent::record(&viewport);
                super::unsent::admitted(&viewport).ok_or_else(|| "no session".to_owned())
            }
            #[cfg(not(test))]
            None => Err("the document is closed".to_owned()),
        };
        match requested {
            Ok(id) => {
                scan.request = id;
                scan.pages += 1;
                self.links.scan = Some(scan);
            }
            Err(error) => self.link_notice(
                format!(
                    "Cannot look for #{}: {error}",
                    midnight_core::link_status_text(&scan.fragment)
                ),
                output,
            ),
        }
    }

    /// A page the heading scan asked for.
    pub(super) fn scanned_page(&mut self, page: document::DocumentPage, output: &Mailbox) {
        let Some(mut scan) = self.links.scan.take() else {
            return;
        };
        let found = page.blocks.iter().find_map(|block| {
            (block.kind == document::BLOCK_HEADING
                && scan.slugs.matches(&block.text.text, &scan.fragment))
            .then_some(block.text.source.offset)
        });
        if let Some(source) = found {
            self.request_page(source, false, output);
            return;
        }
        let next = page.source.offset.saturating_add(page.source.length);
        if !page.has_after || next <= page.source.offset {
            self.fragment_missing(&scan.fragment, output);
            return;
        }
        if scan.started.elapsed() >= SCAN_PROGRESS {
            let percent = (next.saturating_mul(100) / self.length.max(1)).min(99);
            self.status(
                format!(
                    "Looking for #{} in the document… {percent}%",
                    midnight_core::link_status_text(&scan.fragment)
                ),
                output,
            );
        }
        self.request_scan(scan, next, output);
    }

    /// The heading scan's page request failed.
    pub(super) fn scan_failed(&mut self, message: String, output: &Mailbox) {
        if let Some(scan) = self.links.scan.take() {
            self.link_notice(
                format!(
                    "Cannot look for #{}: {message}",
                    midnight_core::link_status_text(&scan.fragment)
                ),
                output,
            );
        }
    }

    /// Stop the heading scan and forget a pending look at a local target: a newer action
    /// replaces them.
    pub(super) fn cancel_link_work(&mut self) {
        if let Some(scan) = self.links.scan.take()
            && let Some(session) = &self.session
        {
            session.cancel(scan.request);
        }
        self.links.resolving = None;
    }

    /// Anything the user does other than pointing and resizing ends a heading scan and a
    /// pending look at a local target.
    pub(super) fn end_link_work_on(&mut self, input: &ViewerInput) {
        let pointing = match input {
            ViewerInput::Raster {
                input: ViewerRasterInput::Hover { .. } | ViewerRasterInput::Release { .. },
                ..
            }
            | ViewerInput::Resize(_)
            | ViewerInput::RetainedRaster(_)
            | ViewerInput::LinkOutcome(_) => true,
            // An HTML page's pointer moves and releases, key-ups (of the Enter that
            // followed the link) and focus changes, as its refusal notice counts them.
            ViewerInput::Web { input, .. } => !super::web::user_action(input),
            _ => false,
        };
        if !pointing && (self.links.scan.is_some() || self.links.resolving.is_some()) {
            self.cancel_link_work();
        }
    }
}

/// Paint a frame `ring` pixels wide just inside `[left, top, right, bottom)` in `color`
/// into a `size` premultiplied RGBA raster, clipped to it. Only the frame's pixels inside
/// the raster are looked at, however large the rectangle; returns how many it painted.
fn paint_ring(
    pixels: &mut [u8],
    [width, height]: [u32; 2],
    [left, top, right, bottom]: [i64; 4],
    ring: i64,
    color: [u8; 4],
) -> usize {
    // The full-width top and bottom rows, then the left and right columns between them.
    let inner_top = top.saturating_add(ring).min(bottom);
    let inner_bottom = bottom.saturating_sub(ring).max(inner_top);
    let inner_left = left.saturating_add(ring).min(right);
    let inner_right = right.saturating_sub(ring).max(inner_left);
    let strips = [
        [left, top, right, inner_top],
        [left, inner_bottom, right, bottom],
        [left, inner_top, inner_left, inner_bottom],
        [inner_right, inner_top, right, inner_bottom],
    ];
    let stride = width as usize * 4;
    let mut painted = 0;
    for [x0, y0, x1, y1] in strips {
        let (x0, x1) = (x0.max(0), x1.min(i64::from(width)));
        let (y0, y1) = (y0.max(0), y1.min(i64::from(height)));
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let (start, end) = (x0 as usize * 4, x1 as usize * 4);
        for y in y0 as usize..y1 as usize {
            if let Some(row) = pixels.get_mut(y * stride + start..y * stride + end) {
                row.as_chunks_mut::<4>().0.fill(color);
                painted += (x1 - x0) as usize;
            }
        }
    }
    painted
}

fn push_bounded(history: &mut Vec<Visit>, visit: Visit) {
    if history.len() >= HISTORY {
        history.remove(0);
    }
    history.push(visit);
}

/// What `path` is on disk. A file no viewer claims by name counts as one a viewer shows
/// when its first bytes are text; it opens in the Basic viewer.
fn local_kind(path: &Path, claimed: bool) -> LocalTargetKind {
    match std::fs::metadata(path) {
        Err(_) => LocalTargetKind::Missing,
        Ok(metadata) if metadata.is_dir() => LocalTargetKind::Folder,
        Ok(metadata) if metadata.is_file() => LocalTargetKind::File {
            viewable: claimed || starts_as_text(path),
        },
        Ok(_) => LocalTargetKind::Other,
    }
}

fn starts_as_text(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0; SNIFF_BYTES];
    let mut read = 0;
    while read < head.len() {
        match file.read(&mut head[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(_) => return false,
        }
    }
    !head[..read].contains(&0)
}
