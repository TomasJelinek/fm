//! A metadata Properties summary whose single folder is still counted on the
//! backend: each newer text replaces the shown summary in place and never reopens
//! a dismissed one. Closing or replacing the summary cancels the count.
use super::*;

pub(super) struct Pending {
    request: DesktopRequestId,
    cancel: Cancellation,
    /// The notice generation that shows this request's summary, once shown.
    notice: Option<i32>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Presenter {
    /// Track a sent Properties request; `cancel` stops its folder count. A previous
    /// request's count is cancelled.
    pub(in crate::presenter) fn properties_requested(
        &mut self,
        request: DesktopRequestId,
        cancel: Cancellation,
    ) {
        self.desktop.properties = Some(Pending {
            request,
            cancel,
            notice: None,
        });
    }

    /// The summary of `request` became the shown notice.
    pub(in crate::presenter) fn properties_shown(&mut self, request: DesktopRequestId) {
        let generation = self.desktop.notice_generation;
        if let Some(pending) = self
            .desktop
            .properties
            .as_mut()
            .filter(|pending| pending.request == request)
        {
            pending.notice = Some(generation);
        }
    }

    pub(in crate::presenter) fn properties_updated(
        &mut self,
        request: DesktopRequestId,
        title: String,
        body: String,
    ) {
        let Some(pending) = self
            .desktop
            .properties
            .as_ref()
            .filter(|pending| pending.request == request)
        else {
            return;
        };
        if pending.notice == Some(self.desktop.notice_generation) {
            // Keep the occurrence token: a dismissed notice is never reopened.
            self.desktop.notice = Some((title, body));
        } else {
            // Another notice replaced the summary: stop counting.
            self.desktop.properties = None;
        }
    }

    /// The notice `generation` was closed.
    pub(in crate::presenter) fn properties_closed(&mut self, generation: i32) {
        if self
            .desktop
            .properties
            .as_ref()
            .is_some_and(|pending| pending.notice == Some(generation))
        {
            self.desktop.properties = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(p: &mut Presenter, request: DesktopRequestId, cancel: &Cancellation) {
        p.properties_requested(request, cancel.clone());
        p.desktop
            .inform("/data/photos", "Contents: Calculating…".into());
        p.properties_shown(request);
    }

    #[test]
    fn counted_contents_replace_the_shown_summary_in_place() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        let request = p.desktop.request();
        let cancel = Cancellation::default();
        shown(&mut p, request, &cancel);
        let generation = p.desktop.notice_generation;
        p.properties_updated(request, "/data/photos".into(), "Contents: 3 files".into());
        assert_eq!(p.desktop.notice_generation, generation, "never reopened");
        assert_eq!(p.desktop.notice.as_ref().unwrap().1, "Contents: 3 files");
        assert!(!cancel.is_cancelled());
        // A stale request changes nothing.
        let other = p.desktop.request();
        p.properties_updated(other, "other".into(), "other".into());
        assert_eq!(p.desktop.notice.as_ref().unwrap().1, "Contents: 3 files");
    }

    #[test]
    fn closing_or_replacing_the_summary_cancels_its_count() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        let request = p.desktop.request();
        let cancel = Cancellation::default();
        shown(&mut p, request, &cancel);
        p.properties_closed(p.desktop.notice_generation - 1);
        assert!(
            !cancel.is_cancelled(),
            "another notice's closing is unrelated"
        );
        p.properties_closed(p.desktop.notice_generation);
        assert!(cancel.is_cancelled());

        let request = p.desktop.request();
        let cancel = Cancellation::default();
        shown(&mut p, request, &cancel);
        p.desktop.inform("Another notice", "Keep this".into());
        p.properties_updated(request, "/data/photos".into(), "Contents: 3 files".into());
        assert_eq!(p.desktop.notice.as_ref().unwrap().1, "Keep this");
        assert!(cancel.is_cancelled());

        let request = p.desktop.request();
        let cancel = Cancellation::default();
        shown(&mut p, request, &cancel);
        let next = p.desktop.request();
        p.properties_requested(next, Cancellation::default());
        assert!(
            cancel.is_cancelled(),
            "a new Properties request replaces the count"
        );
    }
}
