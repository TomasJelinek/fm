//! Small presentation strings are copied through a bounded backend clipboard command.
use super::*;

pub(crate) struct PanelDetail {
    pub(crate) identity: String,
    pub(crate) label: &'static str,
    pub(crate) value: String,
}

impl Presenter {
    pub(crate) fn panel_details(&self, pane: PaneId) -> Vec<PanelDetail> {
        let p = &self.panes[pane.index()];
        let archive = p.archive_selection_summary();
        if archive.is_some()
            || !p.selected.is_empty()
            || !p.selection_preview.is_empty()
            || p.selection_deferred
            || p.selection_pending_bulk
            || p.selection_rebase_pending
        {
            let (files, folders, file_bytes, pending) = archive.unwrap_or_else(|| {
                (
                    p.selected.file_count(),
                    p.selected.folder_count(),
                    p.selected.file_bytes(),
                    !p.selection_preview.is_empty()
                        || p.selection_deferred
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
                });
            }
            if folders > 0 {
                let value = if pending {
                    "Folder size updating…".into()
                } else if archive.is_some() {
                    format!("Folder sizes unknown · {folders} unknown")
                } else {
                    p.selected_folder_sizes()
                        .filter(|summary| summary.folders == folders)
                        .map_or_else(
                            || "Folder size updating…".into(),
                            |summary| folder_summary(summary, &self.number_locale),
                        )
                };
                details.push(PanelDetail {
                    identity: format!(
                        "detail:{}:folder-bytes:{}:{value}",
                        p.generation, p.selection_applied.0
                    ),
                    label: "Selected folder bytes",
                    value,
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
            EntryKind::Directory => p.folder_sizes.get(&entry.path).map(|size| {
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
        let Some(field) = self
            .panel_details(pane)
            .into_iter()
            .find(|field| field.identity == token)
        else {
            return false;
        };
        self.queue_clipboard_text(field.value, send)
    }
}

fn folder_summary(summary: SelectedFolderSizes, locale: &NumberLocale) -> String {
    let mut value = if summary.complete == 0 {
        format!("0 of {} folders complete", summary.folders)
    } else {
        let bytes = summary.complete_bytes.map_or_else(
            || "Size overflow".into(),
            |bytes| format!("{} bytes", locale.format_u64(bytes)),
        );
        format!(
            "{bytes} in {} of {} folders",
            summary.complete, summary.folders
        )
    };
    if summary.partial > 0 {
        let bytes = summary.partial_bytes.map_or_else(
            || "Partial size overflow".into(),
            |bytes| format!("≈ {} bytes", locale.format_u64(bytes)),
        );
        value.push_str(&format!(" · {bytes} ({} partial)", summary.partial));
    }
    let unknown = summary
        .folders
        .saturating_sub(summary.complete)
        .saturating_sub(summary.partial);
    if unknown > 0 {
        value.push_str(&format!(" · {unknown} unknown"));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(p.panel_details(PaneId::Left).len(), 3);
        assert!(
            p.panel_details(PaneId::Left)[1]
                .value
                .ends_with("bytes in files")
        );
    }
}
