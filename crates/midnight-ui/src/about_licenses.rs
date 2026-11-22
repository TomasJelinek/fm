//! The About dialog's third-party list and library details, and Midnight Explorer's
//! own license.
//!
//! The list is the current dependency inventory (`docs/dependencies/cargo-licenses.csv`)
//! restricted to the components a release ships for this platform, plus the
//! bundled components outside Cargo that `docs/THIRD_PARTY.md` records. Like the
//! release notice, it conservatively includes build- and test-time libraries. A
//! library's license text and authors come from the backend
//! (`Command::ReadLicenseNotice`); this module reads no files. The program's own
//! license is the repository's `LICENSE`, compiled in.
use crate::{AboutView, LicenseRow, MainWindow, Presenter};
use midnight_core::{Command, LicenseNotice, LicenseNoticeId, LicenseNoticeRequest};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::OnceLock};

const INVENTORY: &str = include_str!("../../../docs/dependencies/cargo-licenses.csv");
/// Midnight Explorer's license: the unmodified GNU GPL version 3 text. The program is
/// licensed under that version or, at the user's option, any later version.
const OWN_LICENSE_TEXT: &str = include_str!("../../../LICENSE");
const OWN_NAME: &str = "Midnight Explorer";
const OWN_LICENSE: &str = "GPL-3.0-or-later";
const OWN_COPYRIGHT: &str = "© 2026 Tomas Jelinek";
const COLUMNS: [&str; 7] = [
    "name",
    "version",
    "license",
    "source",
    "scopes",
    "targets",
    "direct_by",
];

/// The inventory's target label for this platform's release.
#[cfg(windows)]
const TARGET: &str = "Windows";
#[cfg(not(windows))]
const TARGET: &str = "Linux";

/// Inventory scopes a release package ships, and the component each one is.
/// Renderer tests, the ABI conformance project, the public API/SDK workspaces and
/// the macOS filesystem package are not shipped.
const SHIPPED: [(&str, &str); 14] = [
    ("app", "Midnight Explorer"),
    ("native-linux", "native file system plugin"),
    ("native-windows", "native file system plugin"),
    ("network", "network plugin"),
    ("archive", "archive plugin"),
    ("basic", "text and binary viewer"),
    ("source", "source viewer"),
    ("markdown", "Markdown viewer"),
    ("html-viewer", "HTML viewer"),
    ("image-viewer", "image viewer"),
    ("image-decoder", "image decoder"),
    ("pdf-viewer", "PDF viewer"),
    ("thumbnail-image", "image thumbnails"),
    ("thumbnail-video", "video thumbnails and playback"),
];

/// A shipped component outside Cargo, as `docs/THIRD_PARTY.md` records it.
struct Bundled {
    name: &'static str,
    version: &'static str,
    license: &'static str,
    used_by: &'static str,
    authors: &'static str,
    /// Its notices' folder under the release package's `licenses`, if they are there.
    folder: Option<&'static str>,
    /// Where its notices ship.
    notices: &'static str,
}

const BUNDLED: [Bundled; 12] = [
    Bundled {
        name: "Rust standard library",
        version: "1.99.1",
        license: "MIT OR Apache-2.0",
        used_by: "every program in the package",
        authors: "The Rust Project Developers",
        folder: Some("rust"),
        notices: "Release packages keep its notices in licenses/rust/.",
    },
    Bundled {
        name: "FFmpeg",
        version: "9.0.2",
        license: "LGPL-2.1-or-later",
        used_by: "video thumbnails and playback",
        authors: "The FFmpeg developers",
        folder: None,
        notices: "The video thumbnail package keeps FFmpeg's license, its corresponding \
                  source and the exact build configuration in notices/ffmpeg-9.0.2/.",
    },
    Bundled {
        name: "dav1d",
        version: "1.5.4",
        license: "BSD-2-Clause",
        used_by: "AVIF images in the image viewer and image thumbnails",
        authors: "VideoLAN and dav1d authors",
        folder: None,
        notices: "The image viewer and image thumbnail packages keep its license with \
                  their notices.",
    },
    Bundled {
        name: "Chromium Embedded Framework",
        version: "156.0.26",
        license: "BSD-3-Clause",
        used_by: "HTML viewer",
        authors: "Marshall A. Greenblatt; portions Google Inc.",
        folder: None,
        notices: "The HTML viewer package keeps CEF's LICENSE.txt and Chromium's \
                  CREDITS.html, which lists Chromium's separately licensed components, \
                  including LGPL and MPL parts.",
    },
    Bundled {
        name: "Noto Sans fonts",
        version: "",
        license: "OFL-1.1",
        used_by: "text in SVG images (image viewer and image thumbnails)",
        authors: "Google LLC; The Noto Project Authors",
        folder: None,
        notices: "The unmodified Regular, Arabic and Hebrew fonts. The image viewer and \
                  image thumbnail packages keep the SIL Open Font License and the fonts' \
                  copyright notices.",
    },
    Bundled {
        name: "Unicode case-folding data",
        version: "17.0.0",
        license: "Unicode-3.0",
        used_by: "case-insensitive search in the text and binary viewer",
        authors: "Unicode, Inc.",
        folder: None,
        notices: "The text and binary viewer package keeps UNICODE-LICENSE.txt.",
    },
    Bundled {
        name: "Foxit standard PDF fonts",
        version: "",
        license: "BSD-3-Clause",
        used_by: "PDF viewer and image thumbnails, embedded by hayro-font 0.5.0",
        authors: "PDFium Authors",
        folder: None,
        notices: "The PDF viewer and image thumbnail packages keep the fonts' original notice.",
    },
    Bundled {
        name: "Adobe CMaps",
        version: "",
        license: "BSD-3-Clause",
        used_by: "PDF viewer and image thumbnails, embedded by hayro-cmap 0.1.1",
        authors: "Adobe",
        folder: None,
        notices: "The PDF viewer and image thumbnail packages keep the CMaps' original notice.",
    },
    Bundled {
        name: "Tokyo Night palette",
        version: "cdc07ac",
        license: "Apache-2.0",
        used_by: "Tokyo Night theme",
        authors: "folke (tokyonight.nvim)",
        folder: Some("themes/folke"),
        notices: "Release packages keep its license in licenses/themes/folke/.",
    },
    Bundled {
        name: "One Dark palette",
        version: "47bec7a",
        license: "MIT",
        used_by: "One Dark theme",
        authors: "Joshua Dick",
        folder: Some("themes/joshdick"),
        notices: "Release packages keep its license in licenses/themes/joshdick/.",
    },
    Bundled {
        name: "Rosé Pine palette",
        version: "ff48305",
        license: "MIT",
        used_by: "Rosé Pine and Rosé Pine Dawn themes",
        authors: "Rosé Pine",
        folder: Some("themes/rose-pine"),
        notices: "Release packages keep its license in licenses/themes/rose-pine/.",
    },
    Bundled {
        name: "Catppuccin palette",
        version: "d09787d",
        license: "MIT",
        used_by: "Catppuccin Latte theme",
        authors: "Catppuccin",
        folder: Some("themes/catppuccin"),
        notices: "Release packages keep its license in licenses/themes/catppuccin/.",
    },
];

/// One row of the About list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Library {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) license: String,
    /// The shipped components that use it.
    pub(crate) used_by: String,
    bundled: Option<usize>,
}

impl Library {
    fn bundled(&self) -> Option<&'static Bundled> {
        self.bundled.map(|index| &BUNDLED[index])
    }

    fn title(&self) -> String {
        if self.version.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.version)
        }
    }

    /// The backend request for its notices, or `None` when none ship with the program.
    fn request(&self, id: LicenseNoticeId) -> Option<LicenseNoticeRequest> {
        let folder = match self.bundled() {
            Some(bundled) => Some(bundled.folder?.to_owned()),
            None => None,
        };
        Some(LicenseNoticeRequest {
            id,
            name: self.name.clone(),
            version: self.version.clone(),
            folder,
        })
    }
}

/// Splits one CSV record; quoted fields may hold commas and doubled quotes.
fn fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    out.push(field);
    out
}

/// The libraries a release for `target` ships, one row per name and version with
/// the components that use it, plus the bundled components; sorted by name. An
/// inventory with other columns gives no Cargo rows (a test guards the format).
pub(crate) fn libraries_for(inventory: &str, target: &str) -> Vec<Library> {
    let mut lines = inventory.lines();
    let header_matches = lines.next().is_some_and(|header| fields(header) == COLUMNS);
    let mut libraries: Vec<Library> = Vec::new();
    let mut known: HashMap<(String, String), usize> = HashMap::new();
    for line in lines.filter(|_| header_matches) {
        let columns = fields(line);
        let [name, version, license, _source, scopes, targets, _direct] = columns.as_slice() else {
            continue;
        };
        if name.is_empty() || !targets.split("; ").any(|label| label == target) {
            continue;
        }
        let components: Vec<&str> = scopes
            .split("; ")
            .filter_map(|scope| SHIPPED.iter().find(|(shipped, _)| *shipped == scope))
            .map(|(_, component)| *component)
            .collect();
        if components.is_empty() {
            continue;
        }
        let index = *known
            .entry((name.clone(), version.clone()))
            .or_insert_with(|| {
                libraries.push(Library {
                    name: name.clone(),
                    version: version.clone(),
                    license: license.clone(),
                    used_by: String::new(),
                    bundled: None,
                });
                libraries.len() - 1
            });
        let library = &mut libraries[index];
        for component in components {
            if !library.used_by.split(", ").any(|known| known == component) {
                if !library.used_by.is_empty() {
                    library.used_by.push_str(", ");
                }
                library.used_by.push_str(component);
            }
        }
    }
    libraries.extend(BUNDLED.iter().enumerate().map(|(index, bundled)| Library {
        name: bundled.name.into(),
        version: bundled.version.into(),
        license: bundled.license.into(),
        used_by: bundled.used_by.into(),
        bundled: Some(index),
    }));
    libraries.sort_by_cached_key(|library| (library.name.to_lowercase(), library.version.clone()));
    libraries
}

/// This platform's list, parsed once.
fn libraries() -> &'static [Library] {
    static LIBRARIES: OnceLock<Vec<Library>> = OnceLock::new();
    LIBRARIES.get_or_init(|| libraries_for(INVENTORY, TARGET))
}

/// Indices of the libraries whose name, license or component contains `query`.
pub(crate) fn matching(libraries: &[Library], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    libraries
        .iter()
        .enumerate()
        .filter(|(_, library)| {
            query.is_empty()
                || [&library.name, &library.license, &library.used_by]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&query))
        })
        .map(|(index, _)| index)
        .collect()
}

fn show(view: &AboutView, query: &str) {
    let all = libraries();
    let shown = matching(all, query);
    let rows: Vec<LicenseRow> = shown
        .iter()
        .map(|&index| LicenseRow {
            name: all[index].name.as_str().into(),
            version: all[index].version.as_str().into(),
            license: all[index].license.as_str().into(),
        })
        .collect();
    view.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
    view.set_summary(
        if shown.len() == all.len() {
            format!("{} libraries and components", all.len())
        } else {
            format!("{} of {} libraries and components", shown.len(), all.len())
        }
        .into(),
    );
}

/// The detail request the About dialog awaits and the backend's answer. The
/// Presenter owns it, so the answer arrives with the other backend events.
#[derive(Debug, Default)]
pub(crate) struct NoticeState {
    next: u64,
    awaited: Option<(LicenseNoticeId, usize)>,
    answer: Option<Box<LicenseNotice>>,
}

impl NoticeState {
    fn begin(&mut self, library: usize) -> LicenseNoticeId {
        self.next += 1;
        let id = LicenseNoticeId(self.next);
        self.awaited = Some((id, library));
        self.answer = None;
        id
    }

    fn cancel(&mut self) {
        self.awaited = None;
        self.answer = None;
    }

    /// Keep the answer to the awaited request; drop answers to older ones.
    pub(crate) fn receive(&mut self, notice: Box<LicenseNotice>) {
        if self.awaited.is_some_and(|(id, _)| id == notice.id) {
            self.answer = Some(notice);
        }
    }
}

/// The heading of the authors block and its lines, one per author or copyright
/// statement. `None` while the notice loads: no heading, and the block is hidden.
fn authors_block(authors: Option<&[String]>) -> (&'static str, Vec<SharedString>) {
    match authors {
        None => ("", Vec::new()),
        Some([]) => ("Authors: not stated in its license files.", Vec::new()),
        Some(authors) => (
            "Authors",
            authors
                .iter()
                .map(|author| SharedString::from(author.as_str()))
                .collect(),
        ),
    }
}

fn show_authors(view: &AboutView, authors: Option<&[String]>) {
    let (heading, lines) = authors_block(authors);
    view.set_detail_authors_heading(heading.into());
    view.set_detail_authors(ModelRc::from(Rc::new(VecModel::from(lines))));
}

/// A bundled component's authors, one per line.
fn bundled_authors(bundled: &Bundled) -> Vec<String> {
    bundled.authors.split("; ").map(str::to_owned).collect()
}

/// The detail text when its notices are not installed with the program.
fn missing_text(library: &Library) -> String {
    match library.bundled() {
        Some(bundled) => bundled.notices.into(),
        None => format!(
            "The license text of {} is not installed with this build. Release packages keep \
             the license and copyright notices of the libraries the program uses in the \
             licenses folder beside it; plugin packages keep their own. The declared \
             license is {}.",
            library.title(),
            library.license
        ),
    }
}

/// Show an answered notice once; later refreshes find nothing left to load.
pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.about_notice;
    let (Some((id, index)), Some(answer)) = (state.awaited, state.answer.as_deref()) else {
        return;
    };
    let view = window.global::<AboutView>();
    let Some(library) = libraries().get(index) else {
        return;
    };
    if !view.get_detail_loading() || answer.id != id {
        return;
    }
    view.set_detail_loading(false);
    let authors = match library.bundled() {
        Some(bundled) => bundled_authors(bundled),
        None => answer.authors.clone(),
    };
    show_authors(&view, Some(&authors));
    let mut text = if answer.text.is_empty() {
        missing_text(library)
    } else {
        answer.text.clone()
    };
    if answer.truncated {
        text.push_str("\n\n[Longer notices are cut short here.]");
    }
    view.set_detail_text(text.into());
}

/// Fill the detail view for `library` and ask the backend for its notices.
fn open(
    window: &MainWindow,
    presenter: &RefCell<Presenter>,
    send: &dyn Fn(Command) -> bool,
    index: usize,
) {
    let library = &libraries()[index];
    // The detail replaces the list and its filter: retire the filter's hint first.
    crate::tooltip::dismiss(window.window());
    let view = window.global::<AboutView>();
    view.set_detail_title(library.title().into());
    view.set_detail_license(library.license.as_str().into());
    view.set_detail_used_by(library.used_by.as_str().into());
    view.set_detail_loading(false);
    show_authors(&view, None);
    let request = presenter.try_borrow_mut().ok().map(|mut state| {
        let id = state.about_notice.begin(index);
        let request = library.request(id);
        if request.is_none() {
            state.about_notice.cancel();
        }
        request
    });
    match request {
        Some(Some(request)) => {
            view.set_detail_loading(true);
            view.set_detail_text("Loading the license text…".into());
            if !send(Command::ReadLicenseNotice(Box::new(request))) {
                view.set_detail_loading(false);
                view.set_detail_text(UNAVAILABLE.into());
                if let Ok(mut state) = presenter.try_borrow_mut() {
                    state.about_notice.cancel();
                }
            }
        }
        // Its notices ship with a plugin package, not beside the program.
        Some(None) => {
            let authors = library.bundled().map(bundled_authors).unwrap_or_default();
            show_authors(&view, Some(&authors));
            view.set_detail_text(missing_text(library).into());
        }
        None => view.set_detail_text(UNAVAILABLE.into()),
    }
    view.set_detail_open(true);
}

/// Show Midnight Explorer's own license in the detail view. Its text is compiled in,
/// so nothing is requested; an awaited library notice is dropped.
fn open_own(window: &MainWindow, presenter: &RefCell<Presenter>) {
    // The detail replaces the About tab: retire its View license hint first.
    crate::tooltip::dismiss(window.window());
    if let Ok(mut state) = presenter.try_borrow_mut() {
        state.about_notice.cancel();
    }
    let view = window.global::<AboutView>();
    view.set_detail_title(OWN_NAME.into());
    view.set_detail_license(OWN_LICENSE.into());
    view.set_detail_used_by(SharedString::new());
    view.set_detail_loading(false);
    view.set_detail_authors_heading("Copyright".into());
    view.set_detail_authors(ModelRc::from(Rc::new(VecModel::from(vec![
        SharedString::from(OWN_COPYRIGHT),
    ]))));
    view.set_detail_text(OWN_LICENSE_TEXT.into());
    view.set_detail_open(true);
}

const UNAVAILABLE: &str = "The license text could not be requested. Go back and try again.";

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let view = window.global::<AboutView>();
    show(&view, &view.get_query());
    let weak = window.as_weak();
    view.on_filter(move |query| {
        if let Some(window) = weak.upgrade() {
            let view = window.global::<AboutView>();
            // The rows change, so no row is selected until the user picks one.
            view.set_current(-1);
            show(&view, &query);
        }
    });
    let weak = window.as_weak();
    view.on_retire_hints(move || {
        if let Some(window) = weak.upgrade() {
            crate::tooltip::dismiss(window.window());
        }
    });
    let weak = window.as_weak();
    let own_presenter = Rc::clone(&presenter);
    view.on_open_own_license(move || {
        if let Some(window) = weak.upgrade() {
            open_own(&window, &own_presenter);
        }
    });
    let weak = window.as_weak();
    view.on_open(move |name, version| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if let Some(index) = libraries().iter().position(|library| {
            library.name == name.as_str() && library.version == version.as_str()
        }) {
            open(&window, &presenter, send.as_ref(), index);
        }
    });
}

#[cfg(test)]
#[path = "about_licenses_tests.rs"]
mod tests;
