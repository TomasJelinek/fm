//! Short Find callbacks. The comparison worker scans and positions matching source text.
use super::{Input, OpenWindow};
use midnight_compare_engine::FindQuery;

impl OpenWindow {
    pub(super) fn find(&self, backwards: bool) {
        if self.window.get_choosing_files()
            || self.window.get_image_mode()
            || self.window.get_bytes_mode()
        {
            return;
        }
        if self.window.get_query().is_empty() {
            self.window.invoke_focus_search();
            return;
        }
        let revision = self.next_find_revision();
        self.window.set_search_summary("Searching…".into());
        let query = FindQuery {
            text: self.window.get_query().to_string(),
            case_sensitive: self.window.get_case_sensitive(),
            whole_words: self.window.get_whole_words(),
        };
        let repeated = self.submitted_find.borrow().as_ref() == Some(&query);
        if repeated {
            self.send(Input::FindStep {
                delta: if backwards { -1 } else { 1 },
                revision,
            });
        } else {
            *self.submitted_find.borrow_mut() = Some(query.clone());
            self.send(Input::Find {
                query,
                backwards,
                revision,
            });
        }
    }

    /// Editing invalidates the previous result immediately, without a scan per keystroke.
    pub(super) fn clear_find(&self) {
        self.submitted_find.borrow_mut().take();
        let revision = self.next_find_revision();
        self.window.set_search_summary("".into());
        self.send(Input::Find {
            query: FindQuery::default(),
            backwards: false,
            revision,
        });
    }

    pub(super) fn reset_find(&self) {
        self.submitted_find.borrow_mut().take();
        self.pending
            .borrow_mut()
            .retain(|input| !matches!(input, Input::Find { .. } | Input::FindStep { .. }));
        self.find_revision.set(0);
        self.window.set_query("".into());
        self.window.set_search_summary("".into());
        self.window.set_search_visible(false);
    }

    fn next_find_revision(&self) -> u64 {
        let revision = self.find_revision.get().saturating_add(1);
        self.find_revision.set(revision);
        revision
    }
}

#[cfg(test)]
mod tests {
    use crate::identity_tests::{frame, open, platform, runtime};
    use midnight_compare_engine::{IdentityVerdict, Output};
    use std::sync::Arc;

    #[test]
    fn query_state_coalesces_but_relative_steps_accumulate_and_reset_on_new_comparison() {
        use crate::{Input, queue_input};
        use midnight_compare_engine::FindQuery;
        let mut pending = Vec::new();
        queue_input(
            &mut pending,
            Input::Find {
                query: FindQuery {
                    text: "first".into(),
                    ..FindQuery::default()
                },
                backwards: false,
                revision: 1,
            },
        );
        for (delta, revision) in [(1, 2), (1, 3), (-1, 4)] {
            queue_input(&mut pending, Input::FindStep { delta, revision });
        }
        assert!(matches!(
            &pending[..],
            [
                Input::Find { .. },
                Input::FindStep {
                    delta: 1,
                    revision: 4
                }
            ]
        ));
        queue_input(
            &mut pending,
            Input::Find {
                query: FindQuery::default(),
                backwards: false,
                revision: 5,
            },
        );
        assert!(matches!(&pending[..], [Input::Find { revision: 5, .. }]));

        let _native = platform();
        let runtime = runtime();
        let open = open(&runtime);
        open.present_output(&frame(5, IdentityVerdict::Different));
        *open.pending.borrow_mut() = pending;
        queue_input(
            &mut open.pending.borrow_mut(),
            Input::FindStep {
                delta: 2,
                revision: 6,
            },
        );
        open.find_revision.set(6);
        open.window.set_query("old query".into());
        assert!(open.send(Input::Compare(crate::identity_tests::paths())));
        assert!(
            !open
                .pending
                .borrow()
                .iter()
                .any(|input| matches!(input, Input::Find { .. } | Input::FindStep { .. }))
        );
        assert_eq!(open.find_revision.get(), 0);
        assert!(open.submitted_find.borrow().is_none());
        open.present_output(&frame(5, IdentityVerdict::Different));
        assert!(open.window.get_search_summary().is_empty());
        open.present_output(&frame(6, IdentityVerdict::Different));
        assert!(open.window.get_query().is_empty());
        assert!(open.window.get_search_summary().is_empty());
        open.close();
    }

    #[test]
    fn find_typed_before_first_image_frame_cannot_prevent_the_image_from_opening() {
        let _native = platform();
        let runtime = runtime();
        let open = open(&runtime);
        open.window.set_search_visible(true);
        open.window.set_query("early edit".into());
        open.clear_find();
        assert!(open.find_revision.get() > 0);
        let mut output = frame(5, IdentityVerdict::Pending);
        if let Output::Frame { frame, .. } = &mut output {
            frame.image = true;
        }
        open.present_output(&output);
        assert!(open.window.get_image_mode());
        assert_eq!(open.window.get_surface().size().width, 2);
        assert!(!open.window.get_search_visible());
        assert!(open.window.get_query().is_empty());
        assert_eq!(open.find_revision.get(), 0);
        open.close();
    }

    #[test]
    fn editing_clear_and_close_reject_unread_find_frames_from_earlier_revisions() {
        let _native = platform();
        let runtime = runtime();
        let open = open(&runtime);
        let mut output = frame(5, IdentityVerdict::Different);
        if let Output::Frame { frame, .. } = &mut output {
            let find = Arc::make_mut(&mut frame.find);
            find.revision = 1;
            find.message = "Match 1 of 8 · File A · line 4".into();
        }
        open.find_revision.set(1);
        open.present_output(&output);
        assert!(open.window.get_search_summary().contains("Match 1 of 8"));
        open.clear_find();
        assert_eq!(open.find_revision.get(), 2);
        assert!(open.window.get_search_summary().is_empty());
        open.present_output(&output);
        assert!(open.window.get_search_summary().is_empty());
        if let Output::Frame { frame, .. } = &mut output {
            let find = Arc::make_mut(&mut frame.find);
            find.revision = 2;
            find.message.clear();
        }
        open.present_output(&output);
        assert!(open.window.get_search_summary().is_empty());
        open.close();
        open.present_output(&output);
        assert!(open.window.get_search_summary().is_empty());
    }
}
