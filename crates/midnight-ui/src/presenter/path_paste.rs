//! Bounded clipboard replies. Native editor identities remain in the UI bridge.
use super::*;

#[derive(Default)]
pub(crate) struct State {
    slots: [Option<Slot>; 2],
}
struct Slot {
    id: DesktopRequestId,
    result: Option<Result<Arc<str>, FsError>>,
}

impl Presenter {
    pub(crate) fn path_paste_pending(&self, pane: PaneId, id: DesktopRequestId) -> bool {
        self.path_paste.slots[pane.index()]
            .as_ref()
            .is_some_and(|slot| slot.id == id)
    }
    pub(crate) fn request_path_paste(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> Option<DesktopRequestId> {
        self.cancel_path_paste(pane);
        let id = self.desktop.request();
        if !send(Command::ReadPathClipboardText { request: id }) {
            self.desktop.report(
                "Cannot paste folder path",
                "The clipboard queue is busy. Try pasting again.".into(),
            );
            return None;
        }
        self.path_paste.slots[pane.index()] = Some(Slot { id, result: None });
        Some(id)
    }
    pub(crate) fn cancel_path_paste(&mut self, pane: PaneId) {
        if let Some(slot) = self.path_paste.slots[pane.index()].take() {
            self.retired.push(Box::new(slot));
        }
    }
    pub(crate) fn take_path_paste(
        &mut self,
        pane: PaneId,
        id: DesktopRequestId,
    ) -> Option<Result<Arc<str>, FsError>> {
        let slot = self.path_paste.slots[pane.index()].as_mut()?;
        if slot.id != id {
            return None;
        }
        let result = slot.result.take()?;
        self.path_paste.slots[pane.index()] = None;
        Some(result)
    }
    pub(crate) fn apply_path_paste(
        &mut self,
        id: DesktopRequestId,
        result: Result<Arc<str>, FsError>,
    ) {
        if let Some(slot) = self
            .path_paste
            .slots
            .iter_mut()
            .flatten()
            .find(|s| s.id == id && s.result.is_none())
        {
            slot.result = Some(result);
        } else {
            self.retired.push(Box::new(result));
        }
    }
    pub(crate) fn path_paste_failure(&mut self, message: String) {
        self.desktop.report("Cannot paste folder path", message);
    }
    pub(crate) fn retire_pasted_text(&mut self, text: Arc<str>) {
        self.retired.push(Box::new(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replies_are_single_use_superseded_and_cancelled_by_exact_request() {
        let mut p = Presenter::new(PathBuf::from("left"), PathBuf::from("right"));
        let first = p.request_path_paste(PaneId::Left, &|_| true).unwrap();
        let second = p.request_path_paste(PaneId::Left, &|_| true).unwrap();
        p.apply(Event::PathClipboardTextRead {
            request: first,
            result: Ok(Arc::from("old/")),
        });
        assert!(p.take_path_paste(PaneId::Left, second).is_none());
        p.apply(Event::PathClipboardTextRead {
            request: second,
            result: Ok(Arc::from("new/")),
        });
        assert_eq!(
            &*p.take_path_paste(PaneId::Left, second).unwrap().unwrap(),
            "new/"
        );
        assert!(p.take_path_paste(PaneId::Left, second).is_none());
        let third = p.request_path_paste(PaneId::Right, &|_| true).unwrap();
        p.cancel_path_paste(PaneId::Right);
        p.apply(Event::PathClipboardTextRead {
            request: third,
            result: Ok(Arc::from("late/")),
        });
        assert!(p.take_path_paste(PaneId::Right, third).is_none());
        assert!(p.request_path_paste(PaneId::Left, &|_| false).is_none());
        assert!(
            p.desktop
                .notice
                .as_ref()
                .is_some_and(|(_, body)| body.contains("busy"))
        );
        let closing = p.request_path_paste(PaneId::Left, &|_| true).unwrap();
        p.retire_all();
        p.apply(Event::PathClipboardTextRead {
            request: closing,
            result: Ok(Arc::from("after-close/")),
        });
        assert!(!p.path_paste_pending(PaneId::Left, closing));
        assert!(p.take_path_paste(PaneId::Left, closing).is_none());
    }
}
