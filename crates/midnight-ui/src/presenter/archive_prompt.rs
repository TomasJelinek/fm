//! When every archive opened from a native file that may be open is open, the panel
//! offers to close one instead of only reporting the refusal: one action per archive that
//! tabs or viewers show, then the refused archive opens once more in its place
//! (`Command::ArchiveNavigate::replaces`). Archives that no tab or viewer shows (still
//! closing, or held by a running job or another reader) are named without an action.
use super::*;
use std::rc::Rc;

#[derive(Default)]
pub(crate) struct State {
    /// The sessions that refused the latest open, until its failed listing arrives.
    refused: Option<Refused>,
    offer: Option<Offer>,
    retry: Option<Retry>,
    /// Closes the viewers that show an archive session (`ViewerWindows::close_archive`).
    close_viewers: Option<Rc<dyn Fn(u64)>>,
}

struct Refused {
    pane: PaneId,
    navigation: NavigationId,
    sessions: Vec<ArchiveSession>,
    retry: ArchiveAction,
}

/// The offer of a refused open while its notice shows.
struct Offer {
    generation: i32,
    pane: PaneId,
    tab: TabId,
    navigation: NavigationId,
    retry: ArchiveAction,
    /// The archives that may be closed, in the notice's order.
    closable: Vec<Closable>,
}

struct Closable {
    session: u64,
    container: PathBuf,
    /// The tabs that show it: each leaves for the folder that contains the archive.
    tabs: Vec<(PaneId, TabId)>,
    /// Whether viewers show one of its members: they close.
    viewers: bool,
}

/// The refused open runs again once the chosen archive's holders let go of it.
struct Retry {
    pane: PaneId,
    tab: TabId,
    navigation: NavigationId,
    action: ArchiveAction,
    session: u64,
    /// The panels whose shown tab leaves the archive: the open waits until their listings
    /// end.
    after: Vec<(PaneId, NavigationId)>,
}

/// The file name of an archive, as the notice shows it.
fn name_of(container: &Path) -> String {
    container
        .file_name()
        .unwrap_or(container.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn target_of(action: &ArchiveAction) -> String {
    match action {
        ArchiveAction::Open { path, .. } => name_of(path),
        ArchiveAction::List(location) => name_of(&location.address.container),
        ArchiveAction::OpenMember { parent, .. } => parent.name.clone(),
    }
}

fn panel(pane: PaneId) -> &'static str {
    match pane {
        PaneId::Left => "left panel",
        PaneId::Right => "right panel",
    }
}

/// "a", "a and b", "a, b and c".
fn joined(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl Presenter {
    /// Where viewers that show an archive session are closed (`ViewerWindows`).
    pub fn set_archive_viewer_closer(&mut self, close: Rc<dyn Fn(u64)>) {
        self.archive_prompt.close_viewers = Some(close);
    }

    /// An open was refused because every archive that may be open is open
    /// (`Event::ArchiveSessionsFull`); its failed listing follows. A background re-open
    /// after a failed listing offers nothing: its failure is reported, and the panel
    /// returns to the folder that contains the archive.
    pub(super) fn archive_sessions_full(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        sessions: Vec<ArchiveSession>,
        retry: ArchiveAction,
    ) {
        let p = &self.panes[pane.index()];
        if p.generation != navigation.0
            || p.archive
                .as_ref()
                .is_some_and(archives::ArchivePane::reopening)
        {
            self.retired.push(Box::new((sessions, retry)));
            return;
        }
        let refused = Refused {
            pane,
            navigation,
            sessions,
            retry,
        };
        self.retired
            .push(Box::new(self.archive_prompt.refused.replace(refused)));
    }

    /// After the failed listing of a refused open: the notice that offers to close one
    /// of the open archives. It replaces the panel's error line for this refusal.
    pub(super) fn offer_archive_close(&mut self, pane: PaneId, navigation: NavigationId) {
        let Some(refused) = self
            .archive_prompt
            .refused
            .take_if(|refused| refused.pane == pane && refused.navigation == navigation)
        else {
            return;
        };
        let viewers = self.viewer_archive_sessions();
        let mut closable = Vec::new();
        let mut in_use = Vec::new();
        for session in &refused.sessions {
            let tabs = self.archive_tabs(session.id);
            let viewer_count = viewers.iter().filter(|id| **id == session.id).count();
            if tabs.is_empty() && viewer_count == 0 {
                in_use.push(name_of(&session.container));
                continue;
            }
            let single = |pane: PaneId| self.tab_count(pane) == 1;
            let mut places: Vec<String> = tabs
                .iter()
                .map(|(pane, number, _)| {
                    if single(*pane) {
                        panel(*pane).to_owned()
                    } else {
                        format!("{}, tab {number}", panel(*pane))
                    }
                })
                .collect();
            match viewer_count {
                0 => {}
                1 => places.push("a viewer".into()),
                count => places.push(format!("{count} viewers")),
            }
            closable.push((
                format!(
                    "Close {} ({})",
                    name_of(&session.container),
                    joined(&places)
                ),
                Closable {
                    session: session.id,
                    container: session.container.clone(),
                    tabs: tabs.iter().map(|(pane, _, tab)| (*pane, *tab)).collect(),
                    viewers: viewer_count > 0,
                },
            ));
        }
        let target = target_of(&refused.retry);
        let mut body = if closable.is_empty() {
            match in_use.len() {
                0 => format!("Open {target} again once one of them is closed."),
                1 => format!(
                    "{} is still closing or in use, for example by a running job. Open \
                     {target} again once it has closed.",
                    in_use[0]
                ),
                _ => format!(
                    "{} are still closing or in use, for example by running jobs. Open \
                     {target} again once one of them has closed.",
                    joined(&in_use)
                ),
            }
        } else {
            format!("Close one of them to open {target}.")
        };
        if !closable.is_empty() {
            for name in &in_use {
                body.push_str(&format!("\n{name} is still closing or in use."));
            }
        }
        self.desktop.inform("Two archives are already open", body);
        let (labels, closable): (Vec<_>, Vec<_>) = closable.into_iter().unzip();
        // With nothing to choose, Close is the only answer; otherwise it cancels.
        self.desktop.notice_cancel = !labels.is_empty();
        self.desktop.notice_choices = labels;
        self.archive_prompt.offer = Some(Offer {
            generation: self.desktop.notice_generation,
            pane,
            tab: self.active_tab(pane),
            navigation,
            retry: refused.retry,
            closable,
        });
        let p = &self.panes[pane.index()];
        self.panel_warnings.modal_errors[pane.index()] = Some((p.generation, p.error_revision));
    }

    /// The notice of `generation` closed without a choice: everything stays as it was.
    pub(super) fn archive_offer_closed(&mut self, generation: i32) {
        if self
            .archive_prompt
            .offer
            .as_ref()
            .is_some_and(|offer| offer.generation == generation)
        {
            self.retired
                .push(Box::new(self.archive_prompt.offer.take()));
        }
    }

    /// Choice `index` of the notice of `generation`: its archive's tabs leave it for the
    /// folder that contains it, with the cursor on the archive, its viewers close, and
    /// the refused open runs once more in its place.
    pub(crate) fn choose_notice(
        &mut self,
        generation: i32,
        index: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(mut offer) = self
            .archive_prompt
            .offer
            .take_if(|offer| offer.generation == generation)
        else {
            return false;
        };
        // A later notice replaced this one: its offer is no longer shown.
        if generation != self.desktop.notice_generation || index >= offer.closable.len() {
            self.retired.push(Box::new(offer));
            return false;
        }
        let mut chosen = offer.closable.swap_remove(index);
        // Only what still shows the chosen archive leaves it: a panel that went elsewhere
        // while the notice showed (Find is modeless) stays where it is.
        let holding = self.archive_tabs(chosen.session);
        chosen
            .tabs
            .retain(|(pane, tab)| holding.iter().any(|(p, _, t)| p == pane && t == tab));
        chosen.viewers &= self.viewer_archive_sessions().contains(&chosen.session);
        // Nothing that the notice named still shows it: the offer is out of date.
        if chosen.tabs.is_empty() && !chosen.viewers {
            self.retired.push(Box::new(offer));
            return false;
        }
        let mut after = Vec::new();
        for (pane, tab) in &chosen.tabs {
            if *tab == self.active_tab(*pane) {
                if !self.leave_archive_file(*pane, &chosen.container, send) {
                    self.retired.push(Box::new(offer));
                    return false;
                }
                after.push((*pane, NavigationId(self.panes[pane.index()].generation)));
            } else {
                self.leave_parked_archive(*pane, *tab, &chosen.container);
            }
        }
        if chosen.viewers
            && let Some(close) = &self.archive_prompt.close_viewers
        {
            close(chosen.session);
        }
        self.archive_prompt.retry = Some(Retry {
            pane: offer.pane,
            tab: offer.tab,
            navigation: offer.navigation,
            action: offer.retry.clone(),
            session: chosen.session,
            after,
        });
        self.retired.push(Box::new(offer));
        self.flush_archive_retry(send);
        true
    }

    /// The shown tab of `pane` leaves its archive for the folder that contains
    /// `container`, with the cursor on the archive file.
    fn leave_archive_file(
        &mut self,
        pane: PaneId,
        container: &Path,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(folder) = container.parent().map(Path::to_owned) else {
            return false;
        };
        if !self.navigate(pane, folder, None, send) {
            return false;
        }
        let p = &mut self.panes[pane.index()];
        if let Some(request) = &mut p.pending {
            request.wanted_cursor = Some(container.to_owned());
        }
        p.parent_cursor = false;
        true
    }

    /// Run the refused open again once the chosen archive's holders let go of it, if its
    /// tab still shows that refusal. It names the archive as the one it replaces.
    pub(super) fn flush_archive_retry(&mut self, send: &dyn Fn(Command) -> bool) {
        let Some(retry) = &self.archive_prompt.retry else {
            return;
        };
        let leaving = retry.after.iter().any(|(pane, navigation)| {
            let p = &self.panes[pane.index()];
            p.generation == navigation.0 && p.loading
        });
        if leaving {
            return;
        }
        let Some(retry) = self.archive_prompt.retry.take() else {
            return;
        };
        let p = &self.panes[retry.pane.index()];
        if self.active_tab(retry.pane) != retry.tab
            || p.generation != retry.navigation.0
            || p.loading
        {
            self.retired.push(Box::new(retry.action));
            return;
        }
        self.archive_sessions.replace_next(retry.session);
        self.archive_navigate(retry.pane, retry.action, send);
    }
}
