//! Explicit volume requests capture native/virtual context; replies never reopen dismissed notices.
use super::*;

pub(super) struct Pending {
    request: DesktopRequestId,
    pane: PaneId,
    navigation: NavigationId,
    displayed: PathBuf,
    archive: Option<ArchiveAddress>,
    query: PathBuf,
    notice: i32,
    cancel: Cancellation,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Presenter {
    pub(crate) fn drive_info(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.panes[pane.index()].network.is_some() || self.panes[pane.index()].network_request {
            self.desktop.inform(
                "Network Neighborhood",
                "Open a shared folder to inspect its filesystem and capacity.".into(),
            );
            return;
        }
        let current = &self.panes[pane.index()];
        let displayed = current.display_path().to_owned();
        let archive = current
            .archive
            .as_ref()
            .map(|archive| archive.location.address.clone());
        let query = archive
            .as_ref()
            .map_or_else(|| displayed.clone(), |archive| archive.container.clone());
        let navigation = NavigationId(current.generation);
        let request = self.desktop.request();
        let cancel = Cancellation::default();
        if !send(Command::QueryVolumeInformation(VolumeInformationRequest {
            request,
            pane,
            navigation,
            path: query.clone(),
            cancel: cancel.clone(),
        })) {
            self.desktop.report(
                "Drive information",
                "The volume information queue is busy. Try again.".into(),
            );
            return;
        }
        let context = if archive.is_some() {
            "Archive container"
        } else {
            "Folder"
        };
        self.desktop.inform(
            "Drive information",
            format!(
                "{context}: {}\n\nReading native volume information…",
                query.display()
            ),
        );
        self.drive_information = Some(Pending {
            request,
            pane,
            navigation,
            displayed,
            archive,
            query,
            notice: self.desktop.notice_generation,
            cancel,
        });
    }

    pub(super) fn volume_information_loaded(&mut self, loaded: VolumeInformationResult) {
        let Some(pending) = &self.drive_information else {
            self.retired.push(Box::new(loaded));
            return;
        };
        if loaded.request != pending.request {
            self.retired.push(Box::new(loaded));
            return;
        }
        let current = &self.panes[pending.pane.index()];
        let same = loaded.pane == pending.pane
            && loaded.navigation == pending.navigation
            && loaded.path == pending.query
            && current.generation == pending.navigation.0
            && current.display_path() == pending.displayed
            && current
                .archive
                .as_ref()
                .map(|archive| &archive.location.address)
                == pending.archive.as_ref()
            && self.desktop.notice_generation == pending.notice;
        if same {
            let context = if pending.archive.is_some() {
                "Archive container"
            } else {
                "Folder"
            };
            let body = match &loaded.result {
                Ok(information) => crate::drive_information_presentation::body(
                    context,
                    &pending.query,
                    information,
                    &self.number_locale,
                ),
                Err(error) => format!(
                    "{context}: {}\n\n{}",
                    pending.query.display(),
                    crate::error_presentation::describe(error)
                ),
            };
            // Keep the occurrence token unchanged. PopupWindow opens only when this
            // generation changes, so a reply cannot resurrect a dismissed notice.
            self.desktop.notice = Some(("Drive information".into(), body));
            self.desktop.notice_error = loaded.result.is_err();
        }
        self.drive_information = None;
        self.retired.push(Box::new(loaded));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn query(p: &mut Presenter) -> VolumeInformationRequest {
        let captured = RefCell::new(None);
        p.drive_info(PaneId::Left, &|command| {
            let Command::QueryVolumeInformation(request) = command else {
                panic!()
            };
            *captured.borrow_mut() = Some(request);
            true
        });
        captured.into_inner().unwrap()
    }
    fn reply(request: &VolumeInformationRequest) -> VolumeInformationResult {
        VolumeInformationResult {
            request: request.request,
            pane: request.pane,
            navigation: request.navigation,
            path: request.path.clone(),
            result: Ok(VolumeInformation {
                filesystem: Some("MeasuredFS".into()),
                capacity: VolumeCapacity {
                    total: Some(100),
                    free: Some(40),
                    available: Some(10),
                },
                ..Default::default()
            }),
        }
    }

    #[test]
    fn measured_details_replace_loading_in_place_and_use_filesystem_free_for_used_bytes() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        let request = query(&mut p);
        let token = p.desktop.notice_generation;
        assert!(
            p.desktop
                .notice
                .as_ref()
                .unwrap()
                .1
                .contains("Reading native")
        );
        p.volume_information_loaded(reply(&request));
        assert_eq!(
            p.desktop.notice_generation, token,
            "dismissed popup must not reopen"
        );
        let body = &p.desktop.notice.as_ref().unwrap().1;
        assert!(body.contains("MeasuredFS"));
        assert!(body.contains("Used: 60 bytes"));
        assert!(body.contains("Available to you: 10 bytes"));
        assert!(body.contains("Volume label: Unavailable"));
    }

    #[test]
    fn stale_requests_navigation_and_other_notices_do_not_get_overwritten() {
        for scenario in 0..3 {
            let root = std::env::temp_dir();
            let mut p = Presenter::new(root.clone(), root);
            let request = query(&mut p);
            match scenario {
                0 => {
                    query(&mut p);
                }
                1 => {
                    p.panes[0].generation += 1;
                }
                _ => p.desktop.inform("Another notice", "Keep this".into()),
            }
            let before = p.desktop.notice.clone();
            p.volume_information_loaded(reply(&request));
            assert_eq!(p.desktop.notice, before);
        }
    }

    #[test]
    fn archive_container_scope_cannot_publish_after_member_context_changes() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root.clone());
        p.panes[0].archive = Some(archives::ArchivePane::unavailable(
            ArchiveAddress {
                provider: "fixture".into(),
                format: Some(ArchiveFormat::Zip),
                container: root.join("文档.zip"),
                source_version: vec![1],
                components: vec![b"folder".to_vec()],
            },
            root,
        ));
        let request = query(&mut p);
        assert!(request.path.ends_with("文档.zip"));
        assert!(
            p.desktop
                .notice
                .as_ref()
                .unwrap()
                .1
                .contains("Archive container:")
        );
        p.panes[0]
            .archive
            .as_mut()
            .unwrap()
            .location
            .address
            .components
            .push(b"child".to_vec());
        p.volume_information_loaded(reply(&request));
        assert!(
            p.desktop
                .notice
                .as_ref()
                .unwrap()
                .1
                .contains("Reading native")
        );
    }

    #[test]
    fn queue_refusal_leaves_no_pending_volume_operation() {
        let root = std::env::temp_dir();
        let mut p = Presenter::new(root.clone(), root);
        p.drive_info(PaneId::Left, &|_| false);
        assert!(p.drive_information.is_none());
        assert!(p.desktop.notice_error);
    }
}
