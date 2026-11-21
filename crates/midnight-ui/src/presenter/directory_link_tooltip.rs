//! Advisory native link metadata scoped to the current hovered row.
use super::*;

const PENDING: &str = "Reading link target…";
const UNAVAILABLE: &str = "Link target unavailable";

pub(super) struct LinkHover {
    pane: PaneId,
    navigation: NavigationId,
    identity: String,
    path: PathBuf,
    request: Option<DesktopRequestId>,
    text: String,
}

impl Presenter {
    fn directory_link_row(&self, pane: PaneId, identity: &str) -> Option<(NavigationId, &Path)> {
        let panel = &self.panes[pane.index()];
        if self.operations.closing
            || panel.loading
            || !panel.native_capabilities()
            // The hovered row, not the focused one: its rows must be settled.
            || !panel.rows_settled()
        {
            return None;
        }
        let navigation = panel
            .successful_generation
            .filter(|navigation| navigation.0 == panel.generation)?;
        let id = EntryId(identity.rsplit(':').next()?.parse().ok()?);
        let position = *panel.display_index().by_id.get(&id)?;
        let entry = panel.display_entry(position)?;
        // The existing identity includes the tab's navigation, the flat search
        // revision or the inline tree session and revision, and the exact row.
        if entry.kind != EntryKind::DirectoryLink || panel.row_identity(entry) != identity {
            return None;
        }
        Some((navigation, entry.native_path()?))
    }

    pub(crate) fn read_directory_link_target(
        &mut self,
        pane: PaneId,
        identity: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some((navigation, path)) = self.directory_link_row(pane, identity) else {
            return;
        };
        if self
            .desktop
            .directory_link_hover
            .as_ref()
            .is_some_and(|hover| {
                hover.pane == pane
                    && hover.identity == identity
                    && hover.navigation == navigation
                    && hover.path == path
            })
        {
            return;
        }
        let path = path.to_owned();
        let request = self.desktop.request();
        let accepted = send(Command::ReadDirectoryLinkTarget {
            request,
            pane,
            navigation,
            path: path.clone(),
        });
        self.clear_directory_link_hover();
        self.desktop.directory_link_hover = Some(LinkHover {
            pane,
            navigation,
            identity: identity.into(),
            path,
            request: accepted.then_some(request),
            text: if accepted { PENDING } else { UNAVAILABLE }.into(),
        });
    }

    pub(crate) fn directory_link_hover_changed(&mut self, pane: PaneId, identity: &str) -> bool {
        if !self
            .desktop
            .directory_link_hover
            .as_ref()
            .is_some_and(|hover| hover.pane == pane && hover.identity != identity)
        {
            return false;
        }
        self.clear_directory_link_hover();
        true
    }

    fn clear_directory_link_hover(&mut self) {
        if let Some(hover) = self.desktop.directory_link_hover.take() {
            self.retired.push(Box::new(hover));
        }
    }

    fn hover_is_current(&self, hover: &LinkHover) -> bool {
        self.directory_link_row(hover.pane, &hover.identity)
            == Some((hover.navigation, hover.path.as_path()))
    }

    pub(super) fn apply_directory_link_target(
        &mut self,
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        target: Option<PathBuf>,
    ) {
        let current = self
            .desktop
            .directory_link_hover
            .as_ref()
            .is_some_and(|hover| {
                hover.request == Some(request)
                    && hover.pane == pane
                    && hover.navigation == navigation
                    && self.hover_is_current(hover)
            });
        if current && let Some(hover) = &mut self.desktop.directory_link_hover {
            hover.request = None;
            hover.text = target.as_ref().map_or_else(
                || UNAVAILABLE.into(),
                |target| format!("Link target\n{}", target.to_string_lossy()),
            );
        }
        if let Some(target) = target {
            self.retired.push(Box::new(target));
        }
    }

    pub(crate) fn directory_link_tooltip(&self) -> Option<(PaneId, &str, &str)> {
        let hover = self.desktop.directory_link_hover.as_ref()?;
        self.hover_is_current(hover).then_some((
            hover.pane,
            hover.identity.as_str(),
            hover.text.as_str(),
        ))
    }
}
