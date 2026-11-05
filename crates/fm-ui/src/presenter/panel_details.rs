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
        if p.parent_cursor {
            return [("Name", ".."), ("Bytes", "—"), ("Type", "Parent folder")]
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
                    size.bytes.to_string()
                } else {
                    format!("≈ {}", size.bytes)
                }
            }),
            EntryKind::File => metadata.size.map(|size| size.to_string()),
            _ => None,
        };
        let timestamp = metadata
            .modified
            .and_then(crate::time_presentation::modified);
        let date_time = timestamp.as_deref().and_then(|value| value.split_once(' '));
        let mut fields = vec![
            ("Name", entry.display_name.clone()),
            ("Bytes", size.unwrap_or_else(|| "—".into())),
            ("Date", date_time.map_or("—", |(date, _)| date).to_owned()),
            ("Time", date_time.map_or("—", |(_, time)| time).to_owned()),
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
        let Some(field) = self
            .panel_details(pane)
            .into_iter()
            .find(|field| field.identity == token)
        else {
            return false;
        };
        let request = self.desktop.request();
        if send(Command::CopyText {
            request,
            text: field.value,
        }) {
            self.desktop.requests.insert(request);
            true
        } else {
            self.desktop.report(
                "Clipboard queue busy",
                "Try copying the field again.".into(),
            );
            false
        }
    }
}
