//! The worker owns font discovery; UI receives a completed fixed-pitch catalog.
use super::*;
use midnight_compare_engine::FontChoices;
use slint::{ModelRc, SharedString, VecModel};
use std::sync::Arc;

pub(super) fn present(open: &OpenWindow, fonts: Option<&FontChoices>) {
    let Some(fonts) = fonts else {
        return;
    };
    let mut current = open.fonts.borrow_mut();
    if current
        .as_ref()
        .is_none_or(|old| !Arc::ptr_eq(&old.families, &fonts.families))
    {
        let choices = std::iter::once(SharedString::from("System monospace"))
            .chain(
                fonts
                    .families
                    .iter()
                    .map(|name| SharedString::from(name.as_str())),
            )
            .collect::<Vec<_>>();
        open.window
            .set_font_families(ModelRc::new(VecModel::from(choices)));
    }
    let mut requested = open.requested_font.borrow_mut();
    if requested.as_ref() == Some(&fonts.selected) {
        *requested = None;
    }
    let selected = requested.as_deref().unwrap_or(&fonts.selected);
    let index = fonts
        .families
        .iter()
        .position(|name| name == selected)
        .map_or(0, |index| index + 1);
    open.window.set_font_index(index as i32);
    open.window.set_fonts_ready(true);
    *current = Some(fonts.clone());
}

pub(super) fn bind(open: &Rc<OpenWindow>, listener: Rc<RefCell<Option<FontListener>>>) {
    let weak = Rc::downgrade(open);
    open.window.on_change_font(move |index| {
        let Some(open) = weak.upgrade() else {
            return;
        };
        let family = {
            let choices = open.fonts.borrow();
            let Some(choices) = choices.as_ref() else {
                return;
            };
            let family = match index {
                0 => "monospace",
                value if value > 0 => match choices.families.get(value as usize - 1) {
                    Some(name) => name,
                    None => return,
                },
                _ => return,
            };
            if open
                .requested_font
                .borrow()
                .as_deref()
                .unwrap_or(&choices.selected)
                == family
            {
                return;
            }
            family.to_owned()
        };
        open.send(Input::FontFamily(family.clone()));
        *open.requested_font.borrow_mut() = Some(family.clone());
        open.window.set_font_index(index);
        let callback = listener.borrow().clone();
        if let Some(callback) = callback {
            callback(family);
        }
    });
}
