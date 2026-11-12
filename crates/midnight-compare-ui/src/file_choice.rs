//! Short presentation callbacks; native path identities survive lossy display captions.
use super::{Input, OpenWindow};
use midnight_compare_engine::FileChoice;
use midnight_core::PathPickerOwner;
use slint::ComponentHandle;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

#[cfg(all(test, target_os = "linux"))]
#[path = "file_choice_linux_tests.rs"]
mod linux_tests;
#[cfg(all(test, windows))]
#[path = "file_choice_windows_tests.rs"]
mod windows_tests;

pub(crate) type PickerOwner = fn(&slint::Window) -> PathPickerOwner;

impl OpenWindow {
    pub(crate) fn show_file_choice(&self, choice: &FileChoice) {
        self.window
            .set_file_a(choice.paths[0].to_string_lossy().as_ref().into());
        self.window
            .set_file_b(choice.paths[1].to_string_lossy().as_ref().into());
        *self.choice_paths.borrow_mut() = choice.paths.clone();
        self.window
            .set_choice_message(choice.message.as_str().into());
        self.window.set_choice_error(choice.is_error);
        self.window.set_choice_busy(false);
        self.window.set_choosing_files(true);
        self.window.set_surface(slint::Image::default());
        self.window.set_image_mode(false);
        self.window.set_mode(0);
        self.window.set_amount(0.5);
        self.window.set_accessible_text("".into());
        self.window.invoke_focus_files();
    }

    fn chosen_paths(&self) -> [PathBuf; 2] {
        let paths = self.choice_paths.borrow();
        let fields = [self.window.get_file_a(), self.window.get_file_b()];
        std::array::from_fn(|side| edited_path(&paths[side], fields[side].as_str()))
    }

    fn choice_request(&self, request: impl FnOnce([PathBuf; 2]) -> Input) {
        if self.closed.get() || !self.window.get_choosing_files() || self.window.get_choice_busy() {
            return;
        }
        self.window.set_choice_busy(true);
        self.window.set_choice_message("".into());
        self.send(request(self.chosen_paths()));
    }
}

fn edited_path(original: &Path, text: &str) -> PathBuf {
    if text == original.to_string_lossy() {
        original.to_owned()
    } else {
        PathBuf::from(text)
    }
}

pub(crate) fn bind(open: &Rc<OpenWindow>, owner: PickerOwner) {
    let weak = Rc::downgrade(open);
    open.window.on_compare_chosen_files(move || {
        if let Some(open) = weak.upgrade() {
            open.choice_request(Input::Compare);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_browse_file(move |side| {
        if let Some(open) = weak.upgrade()
            && (0..2).contains(&side)
        {
            open.choice_request(|paths| Input::Browse {
                paths,
                side: side as usize,
                owner: owner(open.window.window()),
            });
        }
    });
}
