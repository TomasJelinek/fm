//! Accept backend recovery only for the still-pending native navigation.
use super::*;

impl Presenter {
    pub(super) fn apply_navigation_fallback(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        error: FsError,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &self.panes[pane.index()];
        let Some(request) = p.pending.as_ref().filter(|request| {
            p.generation == navigation.0 && request.navigation == navigation && !p.archive_request
        }) else {
            return;
        };
        let remaining = request
            .fallback_remaining
            .unwrap_or_else(|| request.path.ancestors().skip(1).count().saturating_add(1));
        let retry = remaining > 0 && path.is_absolute() && path != request.path;
        self.apply_with_sender(
            Event::ListingFailed {
                pane,
                navigation,
                error,
            },
            send,
        );
        if retry
            && self.navigate(pane, path, None, send)
            && let Some(request) = &mut self.panes[pane.index()].pending
        {
            request.fallback_remaining = Some(remaining - 1);
        }
    }
}
