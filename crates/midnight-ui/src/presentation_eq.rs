//! Change detection for presentation values that carry Slint images.
//!
//! Slint 1.18.1 never considers an empty `Image` equal, not even to a clone of
//! itself: `ImageInner`'s `PartialEq` has no arm for two absent images. Any
//! struct holding absent artwork (a missing native icon) therefore always looks
//! changed, so comparing with `==` would replace models and dirty properties on
//! every refresh, repainting unchanged windows. Loaded images compare by pixel
//! buffer identity, which is cheap and stays exact here.
use crate::{
    ActionIcons, BookmarkRow, DetailRow, LocationRow, NavigationChoiceRow, OperationRow,
    PathPartRow, TabRow,
};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Two images present the same artwork: the same pixel buffer, or both absent.
pub(crate) fn same_image(left: &Image, right: &Image) -> bool {
    left == right || (absent(left) && absent(right))
}

fn absent(image: &Image) -> bool {
    let size = image.size();
    size.width == 0 || size.height == 0
}

/// Equality for published presentation values, with absent images equal.
pub(crate) trait SamePresentation {
    fn same_presentation(&self, other: &Self) -> bool;
}

impl SamePresentation for Image {
    fn same_presentation(&self, other: &Self) -> bool {
        same_image(self, other)
    }
}

/// Image-free rows keep derived equality.
macro_rules! same_by_eq {
    ($($ty:ty),+ $(,)?) => {
        $(impl SamePresentation for $ty {
            fn same_presentation(&self, other: &Self) -> bool {
                self == other
            }
        })+
    };
}

same_by_eq!(BookmarkRow, DetailRow, PathPartRow, TabRow);

thread_local! {
    // One shared stand-in replaces both sides' images before the remaining
    // fields are compared; clones of it are equal by pixel-buffer identity.
    static STAND_IN: Image = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(1, 1));
}

macro_rules! same_with_images {
    ($ty:ty: $($field:ident),+ $(,)?) => {
        impl SamePresentation for $ty {
            fn same_presentation(&self, other: &Self) -> bool {
                $(if !same_image(&self.$field, &other.$field) {
                    return false;
                })+
                STAND_IN.with(|stand_in| {
                    let (mut left, mut right) = (self.clone(), other.clone());
                    $(
                        left.$field = stand_in.clone();
                        right.$field = stand_in.clone();
                    )+
                    left == right
                })
            }
        }
    };
}

same_with_images!(NavigationChoiceRow: icon);
same_with_images!(LocationRow: icon);
same_with_images!(OperationRow: icon);
same_with_images!(ActionIcons:
    network, back, forward, parent, refresh, open_path, bookmarks, terminal, options, run,
    tab_add, close_tab, remove_bookmark, add_bookmark, help, rename, view, find, edit, copy,
    move_item, new_folder, new_file, delete, user_menu, menu, save_again, done, close,
    properties, folder_sizes, about, exit, sort_ascending, sort_descending,
);

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> Image {
        Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(2, 2))
    }

    #[test]
    fn absent_artwork_is_unchanged_while_slint_equality_reports_a_change() {
        let empty = Image::default();
        assert_ne!(
            empty,
            empty.clone(),
            "pinned Slint behavior this module covers"
        );
        assert!(same_image(&empty, &Image::default()));
        let icon = loaded();
        assert!(same_image(&icon, &icon.clone()));
        assert!(
            !same_image(&icon, &loaded()),
            "different pixel buffers differ"
        );
        assert!(!same_image(&icon, &empty));
    }

    #[test]
    fn rows_with_absent_icons_compare_by_their_other_fields() {
        let row = OperationRow {
            identity: "copy-job:1".into(),
            title: "Copy".into(),
            progress: 0.5,
            ..Default::default()
        };
        assert_ne!(row, row.clone(), "an absent icon defeats derived equality");
        assert!(row.same_presentation(&row.clone()));
        let mut moved = row.clone();
        moved.progress = 0.75;
        assert!(!row.same_presentation(&moved));
        let mut icon = row.clone();
        icon.icon = loaded();
        assert!(
            !row.same_presentation(&icon),
            "arriving artwork is a change"
        );
        assert!(icon.same_presentation(&icon.clone()));

        let palette = ActionIcons::default();
        assert!(palette.same_presentation(&ActionIcons::default()));
        let arrived = ActionIcons {
            sort_descending: loaded(),
            ..Default::default()
        };
        assert!(!palette.same_presentation(&arrived));
        let choice = NavigationChoiceRow {
            identity: "home".into(),
            ..Default::default()
        };
        assert!(choice.same_presentation(&choice.clone()));
        let location = LocationRow {
            identity: "home".into(),
            selected: true,
            ..Default::default()
        };
        let mut deselected = location.clone();
        deselected.selected = false;
        assert!(location.same_presentation(&location.clone()));
        assert!(!location.same_presentation(&deselected));
    }
}
