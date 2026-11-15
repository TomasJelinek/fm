//! Small presentation strings are copied through a bounded backend clipboard command.
use super::*;

pub(crate) struct PanelDetail {
    pub(crate) identity: String,
    pub(crate) label: &'static str,
    pub(crate) value: String,
    pub(crate) explanation: String,
}

impl Presenter {
    pub(crate) fn panel_details(&self, pane: PaneId) -> Vec<PanelDetail> {
        let p = &self.panes[pane.index()];
        if p.network_hides_rows() {
            return Vec::new();
        }
        let rectangle = p.rectangle_summary();
        if rectangle == Some(None) {
            return vec![PanelDetail {
                identity: format!("detail:{}:rectangle-pending", p.generation),
                label: "Selected",
                value: "Updating selection…".into(),
                explanation: String::new(),
            }];
        }
        let archive = rectangle
            .flatten()
            .map(|totals| (totals.files(), totals.folders(), totals.file_bytes(), false))
            .or_else(|| p.archive_selection_summary());
        if archive.is_some()
            || (p.tree_view().is_none()
                && (!p.selected.is_empty()
                    || !p.selection_preview.is_empty()
                    || p.selection_deferred.any()
                    || p.selection_pending_bulk
                    || p.selection_rebase_pending))
        {
            let (files, folders, file_bytes, pending) = archive.unwrap_or_else(|| {
                (
                    p.selected.file_count(),
                    p.selected.folder_count(),
                    p.selected.file_bytes(),
                    !p.selection_preview.is_empty()
                        || p.selection_deferred.any()
                        || p.selection_pending_bulk
                        || p.selection_rebase_pending
                        || p.selection_revision != p.selection_applied,
                )
            });
            let value = format!(
                "{files} {}, {folders} {}{}",
                if files == 1 { "file" } else { "files" },
                if folders == 1 { "folder" } else { "folders" },
                if pending { " (updating)" } else { "" }
            );
            let mut details = vec![PanelDetail {
                identity: format!(
                    "detail:{}:selection:{}:{}:{value}",
                    p.generation, p.selection_applied.0, p.selection_revision.0
                ),
                label: "Selected",
                value,
                explanation: String::new(),
            }];
            if files > 0 {
                let value = if pending {
                    "Size updating…".into()
                } else {
                    file_bytes.map_or_else(
                        || "Size unknown".into(),
                        |bytes| {
                            format!(
                                "{} bytes{}",
                                self.number_locale.format_u64(bytes),
                                if folders > 0 { " in files" } else { "" }
                            )
                        },
                    )
                };
                details.push(PanelDetail {
                    identity: format!(
                        "detail:{}:selection-bytes:{}:{value}",
                        p.generation, p.selection_applied.0
                    ),
                    label: "Selected file bytes",
                    value,
                    explanation: String::new(),
                });
            }
            if folders > 0
                && rectangle.is_none()
                && !pending
                && let Some(value) = p
                    .selected_folder_sizes()
                    .filter(|summary| summary.folders == folders)
                    .and_then(|summary| folder_summary(summary, &self.number_locale))
            {
                details.push(PanelDetail {
                    identity: format!(
                        "detail:{}:folder-bytes:{}:{value}",
                        p.generation, p.selection_applied.0
                    ),
                    label: "Selected folder bytes",
                    value,
                    explanation: String::new(),
                });
            }
            return details;
        }
        if p.parent_cursor {
            return [("Name", ".."), ("Bytes", "—")]
                .into_iter()
                .enumerate()
                .map(|(index, (label, value))| PanelDetail {
                    identity: format!("detail:{}:parent:{index}:{value}", p.generation),
                    label,
                    value: value.into(),
                    explanation: String::new(),
                })
                .collect();
        }
        let Some(entry) = p.cursor.and_then(|id| p.entry_by_id(id)) else {
            return Vec::new();
        };
        if !p.search_allows_activation() {
            return Vec::new();
        }
        let metadata = p
            .metadata
            .get(&entry.id)
            .filter(|_| p.showing_stream())
            .unwrap_or(&entry.metadata);
        let size = match entry.kind {
            EntryKind::Directory | EntryKind::DirectoryLink => p.folder_size(entry).map(|size| {
                if size.finished && size.errors == 0 {
                    self.number_locale.format_u64(size.bytes)
                } else {
                    format!("≈ {}", self.number_locale.format_u64(size.bytes))
                }
            }),
            EntryKind::File => metadata
                .size
                .map(|size| self.number_locale.format_u64(size)),
            _ => None,
        };
        let date_time = crate::time_presentation::modified(metadata);
        let mut fields = vec![
            ("Name", entry.display_name.clone()),
            ("Bytes", size.unwrap_or_else(|| "—".into())),
            (
                "Date",
                date_time
                    .as_ref()
                    .map_or("—", |value| value.date.as_str())
                    .to_owned(),
            ),
            (
                "Time",
                date_time
                    .as_ref()
                    .map_or("—", |value| value.time.as_str())
                    .to_owned(),
            ),
        ];
        fields.extend(crate::detail_attributes::fields(metadata));
        fields
            .into_iter()
            .enumerate()
            .map(|(index, (label, value))| PanelDetail {
                // Including the field value rejects a held click across a metadata update too.
                identity: format!(
                    "detail:{}:{}:{}:{}:{index}:{value}",
                    p.generation,
                    p.displayed_generation().0,
                    p.search.view_revision,
                    entry.id.0
                ),
                label,
                value,
                explanation: if label == "Attributes" {
                    crate::detail_attributes::explanation(metadata)
                } else {
                    String::new()
                },
            })
            .collect()
    }

    pub(crate) fn copy_panel_detail(
        &mut self,
        pane: PaneId,
        token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if let Some(value) = token.strip_prefix("capacity:") {
            return value == crate::volume_presentation::panel_space(self, pane)
                && self.queue_clipboard_text(value.to_owned(), send);
        }
        let (full_path, token) = token
            .strip_prefix("path:")
            .map_or((false, token), |token| (true, token));
        let Some(field) = self
            .panel_details(pane)
            .into_iter()
            .find(|field| field.identity == token)
        else {
            return false;
        };
        if full_path {
            if field.label != "Name" {
                return false;
            }
            let Some(path) = self.focused_full_path(pane) else {
                return false;
            };
            let Some(text) = path.to_str() else {
                self.desktop.report(
                    "Cannot copy full path",
                    "This native path cannot be represented exactly as clipboard text.".into(),
                );
                return false;
            };
            return self.queue_clipboard_text(text.to_owned(), send);
        }
        self.queue_clipboard_text(field.value, send)
    }
}

fn folder_summary(summary: SelectedFolderSizes, locale: &NumberLocale) -> Option<String> {
    let mut parts = Vec::with_capacity(2);
    if summary.complete > 0 {
        let bytes = summary.complete_bytes.map_or_else(
            || "Size overflow".into(),
            |bytes| format!("{} bytes", locale.format_u64(bytes)),
        );
        parts.push(format!(
            "{bytes} in {} of {} folders",
            summary.complete, summary.folders
        ));
    }
    if summary.partial > 0 {
        let bytes = summary.partial_bytes.map_or_else(
            || "Partial size overflow".into(),
            |bytes| format!("≈ {} bytes", locale.format_u64(bytes)),
        );
        parts.push(format!("{bytes} ({} partial)", summary.partial));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focused_status_entry(path: PathBuf) -> Presenter {
        let root = path.parent().unwrap().to_owned();
        let mut p = Presenter::new(root.clone(), root);
        let mut entry = Entry::new(EntryId(7), path, EntryKind::File);
        entry.display_name = "displayed 文档.txt".into();
        entry.metadata.size = Some(32);
        p.panes[0].entries = vec![entry].into();
        p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
        p.panes[0].cursor = Some(EntryId(7));
        p.panes[0].parent_cursor = false;
        p
    }

    #[test]
    fn status_name_full_path_copy_uses_native_identity_and_keeps_plain_copy_unchanged() {
        let path = std::env::temp_dir()
            .join("資料 e\u{301} 🧪")
            .join("native ' $().txt");
        let mut p = focused_status_entry(path.clone());
        let name = p.panel_details(PaneId::Left).remove(0).identity;
        let sent = std::cell::RefCell::new(Vec::new());
        let send = |command| {
            sent.borrow_mut().push(command);
            true
        };
        assert!(p.copy_panel_detail(PaneId::Left, &name, &send));
        assert!(p.copy_panel_detail(PaneId::Left, &format!("path:{name}"), &send));
        let sent = sent.borrow();
        assert!(matches!(&sent[0], Command::CopyText { text, .. } if text == "displayed 文档.txt"));
        assert!(
            matches!(&sent[1], Command::CopyText { text, .. } if text == path.to_str().unwrap())
        );
    }

    #[test]
    fn status_name_full_path_copy_rejects_non_name_stale_parent_and_loading_requests() {
        let mut p = focused_status_entry(std::env::temp_dir().join("native.txt"));
        let details = p.panel_details(PaneId::Left);
        let name = details[0].identity.clone();
        let bytes = details[1].identity.clone();
        assert!(
            !p.copy_panel_detail(PaneId::Left, &format!("path:{bytes}"), &|_| panic!(
                "not a name"
            ))
        );
        p.panes[0].loading = true;
        assert!(
            !p.copy_panel_detail(PaneId::Left, &format!("path:{name}"), &|_| panic!(
                "loading"
            ))
        );
        p.panes[0].loading = false;
        p.panes[0].generation += 1;
        assert!(
            !p.copy_panel_detail(PaneId::Left, &format!("path:{name}"), &|_| panic!(
                "stale name"
            ))
        );
        p.panes[0].parent_cursor = true;
        let parent = p.panel_details(PaneId::Left)[0].identity.clone();
        assert!(
            !p.copy_panel_detail(PaneId::Left, &format!("path:{parent}"), &|_| panic!(
                "parent"
            ))
        );
    }

    #[test]
    fn status_name_full_path_copy_does_not_reinterpret_archive_member_captions() {
        let mut p = focused_status_entry(std::env::temp_dir().join("native.txt"));
        let entry = Entry::from_archive(ArchiveNode {
            id: VirtualEntryId { mount: 1, entry: 7 },
            parent: 0,
            raw_name: b"member.txt".to_vec().into(),
            display_name: "looks-like-a-native-path.txt".into(),
            kind: EntryKind::File,
            size: Some(32),
            readable: true,
            modified: None,
        });
        p.panes[0].entries = vec![entry].into();
        p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
        let name = p.panel_details(PaneId::Left)[0].identity.clone();
        assert!(
            !p.copy_panel_detail(PaneId::Left, &format!("path:{name}"), &|_| panic!(
                "virtual name has no native path"
            ))
        );
    }

    #[cfg(unix)]
    #[test]
    fn status_name_full_path_copy_rejects_lossy_native_bytes() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let path = std::env::temp_dir().join(OsString::from_vec(b"raw-\xff.txt".to_vec()));
        let mut p = focused_status_entry(path);
        let name = p.panel_details(PaneId::Left)[0].identity.clone();
        assert!(
            !p.copy_panel_detail(PaneId::Left, &format!("path:{name}"), &|_| panic!(
                "lossy replacement text is not the native path"
            ))
        );
        assert_eq!(
            p.desktop.notice.as_ref().unwrap().0,
            "Cannot copy full path"
        );
    }

    #[test]
    fn selected_file_bytes_are_exact_and_never_include_folder_estimates() {
        let mut p = Presenter::new("left".into(), "right".into());
        let entries: Vec<_> = [1234, 5678]
            .into_iter()
            .enumerate()
            .map(|(i, size)| {
                let mut entry = Entry::new(
                    EntryId(i as u64),
                    PathBuf::from(format!("left/{i}")),
                    EntryKind::File,
                );
                entry.metadata.size = Some(size);
                entry
            })
            .collect();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        p.panes[0].selected = Arc::new(SelectionSnapshot::from_bits(index, vec![3]).unwrap());
        let expected = format!("{} bytes", p.number_locale.format_u64(6912));
        assert_eq!(p.panel_details(PaneId::Left)[1].value, expected);
        let token = p.panel_details(PaneId::Left)[1].identity.clone();
        assert!(p.copy_panel_detail(PaneId::Left, &token, &|command| {
            assert!(matches!(command, Command::CopyText {text, ..} if text == expected));
            true
        }));
        p.panes[0].selection_pending_bulk = true;
        assert_eq!(p.panel_details(PaneId::Left)[1].value, "Size updating…");
        assert!(!p.copy_panel_detail(PaneId::Left, &token, &|_| panic!("stale field")));
        p.panes[0].selection_pending_bulk = false;
        let mut mixed = entries;
        mixed[1].kind = EntryKind::Directory;
        let index = Arc::new(DirectoryIndex::from_entries(&mixed));
        p.panes[0].selected = Arc::new(SelectionSnapshot::from_bits(index, vec![3]).unwrap());
        assert_eq!(p.panel_details(PaneId::Left).len(), 2);
        assert!(
            p.panel_details(PaneId::Left)[1]
                .value
                .ends_with("bytes in files")
        );
    }
}
