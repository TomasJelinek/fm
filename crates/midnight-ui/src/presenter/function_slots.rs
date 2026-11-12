//! Effective function-key layers are cached with the compiled shortcut map.
use super::{KeyboardBindings, display_key};
use crate::FunctionSlot;
use midnight_core::{ShortcutChord, ShortcutContext, ShortcutMap};

pub(super) fn build(map: &ShortcutMap, context: ShortcutContext) -> [[FunctionSlot; 12]; 16] {
    std::array::from_fn(|modifiers| build_layer(map, context, modifiers))
}

fn build_layer(
    map: &ShortcutMap,
    context: ShortcutContext,
    modifiers: usize,
) -> [FunctionSlot; 12] {
    std::array::from_fn(|index| {
        let chord = ShortcutChord {
            key: format!("F{}", index + 1),
            control: modifiers & 1 != 0,
            alt: modifiers & 2 != 0,
            shift: modifiers & 4 != 0,
            meta: modifiers & 8 != 0,
        };
        let Some(action) = map.resolve(&chord, context) else {
            return FunctionSlot {
                key: chord.display().into(),
                hint: format!(
                    "{} is unassigned. Configure it in Options → Keyboard.",
                    chord.display()
                )
                .into(),
                ..Default::default()
            };
        };
        let (label, icon) = label_icon(&action.id, &action.label);
        let keys = map
            .bindings(&action.id)
            .unwrap()
            .iter()
            .map(display_key)
            .collect::<Vec<_>>()
            .join(" / ");
        FunctionSlot {
            action: action.id.as_str().into(),
            label: label.into(),
            key: chord.display().into(),
            hint: format!(
                "{} ({keys})\n{}",
                action.label,
                action.scope_for(&chord).label()
            )
            .into(),
            icon,
        }
    })
}

fn label_icon<'a>(id: &str, label: &'a str) -> (&'a str, i32) {
    match id {
        "panel.drive-info" => ("Drive info", 12),
        "files.attributes" => ("Attributes", 12),
        "files.case" => ("Case", 1),
        "files.checksums" => ("Checksums", 15),
        "sort.name" => ("Name", 13),
        "sort.extension" => ("Extension", 13),
        "sort.modified" => ("Modified", 13),
        "sort.size" => ("Size", 13),
        "panel.refresh" => ("Refresh", 14),
        "panel.compare" => ("Compare", 2),
        "panel.zoom" => ("Zoom", 2),
        "panel.filter" => ("Filter", 2),
        "panel.folder-sizes" => ("Dir sizes", 12),
        "drive.left" => ("Left drive", 10),
        "drive.right" => ("Right drive", 10),
        "files.rename" => ("Rename", 1),
        "files.view" => ("View", 2),
        "files.view-basic" => ("Text view", 2),
        "files.edit" => ("Edit", 3),
        "files.find" => ("Find", 9),
        "files.create-file" => ("New file", 6),
        "files.copy" | "files.copy-to" => ("Copy", 4),
        "files.move-to" => ("Move", 5),
        "files.paste" => ("Paste", 4),
        "files.create-directory" => ("Create dir", 6),
        "files.trash" => ("Delete", 7),
        "files.delete" => ("Erase", 7),
        "app.help" => ("Help", 0),
        "app.exit" => ("Exit", 9),
        "app.user-menu" => ("User menu", 8),
        "app.main-menu" => ("Menu", 9),
        "network.connect" => ("Connect", 10),
        "network.disconnect" => ("Disconnect", 11),
        _ => (label, 9),
    }
}

impl KeyboardBindings {
    pub(crate) fn function_slot(
        &self,
        index: i32,
        modifiers: i32,
        searching: bool,
    ) -> FunctionSlot {
        let rows = if searching {
            &self.function_search
        } else {
            &self.function_files
        };
        usize::try_from(modifiers)
            .ok()
            .and_then(|modifiers| rows.get(modifiers))
            .and_then(|layer| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| layer.get(index))
            })
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::ShortcutBinding;
    use std::sync::Arc;

    #[test]
    fn remapping_and_disabling_replace_the_exact_control_cell_and_scope() {
        let overrides = [
            ShortcutBinding {
                action: "files.attributes".into(),
                keys: vec![],
            },
            ShortcutBinding {
                action: "files.copy".into(),
                keys: vec!["Ctrl+F2".into()],
            },
            ShortcutBinding {
                action: "panel.zoom".into(),
                keys: vec![],
            },
        ];
        let cache = KeyboardBindings::new(&overrides, &Arc::default(), 1).unwrap();
        let row = cache.function_slot(1, 1, false);
        assert_eq!(row.action, "files.copy");
        assert_eq!(row.label, "Copy");
        assert_eq!(row.key, "Ctrl+F2");
        assert!(row.hint.contains("Ctrl+F2"));
        assert!(cache.function_slot(1, 1, true).action.is_empty());
        assert_eq!(cache.function_slot(7, 1, false).action, "files.checksums");
        assert_eq!(cache.function_slot(7, 1, false).label, "Checksums");
        assert_eq!(cache.function_slot(7, 1, false).icon, 15);
        assert!(cache.function_slot(10, 1, false).action.is_empty());
        assert!(cache.function_slot(-1, 1, false).action.is_empty());
        assert!(cache.function_slot(12, 1, false).action.is_empty());
    }
}
