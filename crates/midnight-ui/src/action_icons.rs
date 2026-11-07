//! Project the bounded native icon cache into named UI action roles.
use crate::{ActionIcons, Presenter};
use midnight_core::{ActionIcon, IconKey};

pub(super) fn palette(presenter: &Presenter) -> ActionIcons {
    let image = |role| presenter.desktop.image(&IconKey::action(role));
    ActionIcons {
        network: image(ActionIcon::Network),
        back: image(ActionIcon::Back),
        forward: image(ActionIcon::Forward),
        parent: image(ActionIcon::Parent),
        refresh: image(ActionIcon::Refresh),
        open_path: image(ActionIcon::OpenPath),
        bookmarks: image(ActionIcon::Bookmarks),
        terminal: image(ActionIcon::Terminal),
        options: image(ActionIcon::Options),
        run: image(ActionIcon::Run),
        tab_add: image(ActionIcon::TabAdd),
        close_tab: image(ActionIcon::CloseTab),
        remove_bookmark: image(ActionIcon::RemoveBookmark),
        add_bookmark: image(ActionIcon::AddBookmark),
        help: image(ActionIcon::Help),
        rename: image(ActionIcon::Rename),
        view: image(ActionIcon::View),
        find: image(ActionIcon::Find),
        edit: image(ActionIcon::Edit),
        copy: image(ActionIcon::Copy),
        move_item: image(ActionIcon::Move),
        new_folder: image(ActionIcon::NewFolder),
        new_file: image(ActionIcon::NewFile),
        delete: image(ActionIcon::Delete),
        user_menu: image(ActionIcon::UserMenu),
        menu: image(ActionIcon::Menu),
        save_again: image(ActionIcon::SaveAgain),
        done: image(ActionIcon::Done),
        close: image(ActionIcon::Close),
        properties: image(ActionIcon::Properties),
        folder_sizes: image(ActionIcon::FolderSizes),
        about: image(ActionIcon::About),
        exit: image(ActionIcon::Exit),
        sort_ascending: image(ActionIcon::SortAscending),
        sort_descending: image(ActionIcon::SortDescending),
    }
}
