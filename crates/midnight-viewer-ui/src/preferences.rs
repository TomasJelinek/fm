//! Small presentation callbacks; validation and persistence belong to services.
use super::*;
use slint::Model;

pub(super) fn present_font_catalog(open: &OpenWindow) {
    if open.font_catalog_ready.get() {
        return;
    }
    let Some(installed) = open.session.font_families() else {
        return;
    };
    let defaults = ["sans-serif", "serif", "monospace"];
    let names: Vec<slint::SharedString> = defaults
        .iter()
        .copied()
        .chain(installed.iter().map(String::as_str).filter(|name| {
            !defaults
                .iter()
                .any(|default| default.eq_ignore_ascii_case(name))
        }))
        .map(Into::into)
        .collect();
    open.window
        .set_font_families(slint::ModelRc::new(slint::VecModel::from(names)));
    open.font_catalog_ready.set(true);
    present(open);
}

fn present_font_family(window: &ViewerWindow, family: &str) {
    let model = window.get_font_families();
    if usize::try_from(window.get_font_family_index())
        .ok()
        .and_then(|index| model.row_data(index))
        .is_some_and(|name| name == family)
    {
        window.set_viewer_font_family(family.into());
        return;
    }
    let index = model
        .iter()
        .position(|name| name == family)
        .unwrap_or_else(|| {
            // A saved font may be unavailable on this machine. Keep its exact name
            // selected until the user explicitly chooses a replacement.
            let mut names: Vec<_> = model.iter().collect();
            let index = names.len();
            names.push(family.into());
            window.set_font_families(slint::ModelRc::new(slint::VecModel::from(names)));
            index
        });
    window.set_viewer_font_family(family.into());
    window.set_font_family_index(index as i32);
}

fn font_kind(window: &ViewerWindow) -> ViewerFontKind {
    if window.get_markdown_mode() {
        ViewerFontKind::Markdown
    } else if window.get_source_mode() {
        ViewerFontKind::Source
    } else {
        ViewerFontKind::Text
    }
}

pub(super) fn present(open: &OpenWindow) {
    let preferences = open.session.preferences();
    let window = &open.window;
    let font = preferences.font(font_kind(window));
    window.set_case_sensitive(preferences.case_sensitive);
    window.set_line_numbers(preferences.line_numbers);
    present_font_family(window, &font.family);
    window.set_viewer_font_size(i32::from(font.size));
    window.set_markdown_preset(match preferences.markdown_preset {
        MarkdownPreset::GitHub => 0,
        MarkdownPreset::Compact => 1,
        MarkdownPreset::Comfortable => 2,
    });
}

pub(super) fn bind(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_change_line_numbers(move |enabled| {
        if let Some(open) = weak.upgrade() {
            accept_preference(&open, ViewerPreferenceChange::LineNumbers(enabled));
            present(&open);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_font_family(move |family| {
        if let Some(open) = weak.upgrade() {
            // A model refresh must not persist a temporary empty selection.
            if family.trim().is_empty() {
                return;
            }
            accept_preference(
                &open,
                ViewerPreferenceChange::FontFamily(font_kind(&open.window), family.to_string()),
            );
            present(&open);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_font_size(move |size| {
        if let Some(open) = weak.upgrade()
            && let Ok(size) = u16::try_from(size)
        {
            accept_preference(
                &open,
                ViewerPreferenceChange::FontSize(font_kind(&open.window), size),
            );
            present(&open);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_markdown_preset(move |preset| {
        let preset = match preset {
            0 => MarkdownPreset::GitHub,
            1 => MarkdownPreset::Compact,
            2 => MarkdownPreset::Comfortable,
            _ => return,
        };
        if let Some(open) = weak.upgrade() {
            accept_preference(&open, ViewerPreferenceChange::MarkdownPreset(preset));
            present(&open);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn arriving_installed_catalog_preserves_named_preference_without_saving_another_edit() {
        let (_runtime, manager, open, _service) = crate::image_presentation_tests::open();
        let changes = Rc::new(RefCell::new(Vec::new()));
        let recorded = changes.clone();
        manager.set_preferences_listener(Rc::new(move |change| recorded.borrow_mut().push(change)));
        open.window.set_markdown_mode(true);
        open.window
            .invoke_change_font_family("Unavailable 字, Display".into());
        let deadline = Instant::now() + Duration::from_secs(10);
        while open.session.font_families().is_none() {
            assert!(Instant::now() < deadline, "font catalog did not arrive");
            std::thread::sleep(Duration::from_millis(5));
        }
        present_font_catalog(&open);
        let names = open.window.get_font_families();
        assert!(
            names.row_count() > 4,
            "installed names must reach the chooser"
        );
        assert_eq!(
            names
                .row_data(open.window.get_font_family_index() as usize)
                .unwrap(),
            "Unavailable 字, Display"
        );
        assert_eq!(
            changes.borrow().as_slice(),
            [ViewerPreferenceChange::FontFamily(
                ViewerFontKind::Markdown,
                "Unavailable 字, Display".into()
            )]
        );
        open.window.set_markdown_mode(false);
        open.window.set_source_mode(true);
        present(&open);
        assert_eq!(
            names
                .row_data(open.window.get_font_family_index() as usize)
                .unwrap(),
            "monospace",
            "switching modes must restore the other font's visible selection"
        );
        assert_eq!(changes.borrow().len(), 1);
        manager.close_all();
    }
}
