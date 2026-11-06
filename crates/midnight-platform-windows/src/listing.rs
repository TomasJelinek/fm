//! Windows classifies link targets' declared type from enumeration data, without following them.
use midnight_core::EntryKind;
use std::{fs::FileType, os::windows::fs::FileTypeExt};

pub(crate) fn classify(file_type: FileType) -> EntryKind {
    if file_type.is_symlink_dir() {
        EntryKind::DirectoryLink
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WindowsFileSystem;
    use midnight_core::{Cancellation, DirectoryService};
    use std::{fs, os::windows::process::CommandExt, path::Path, process::Command};

    #[test]
    fn cached_junction_type_is_a_directory_link_even_after_the_target_disappears() {
        let temp = tempfile::Builder::new()
            .prefix("midnight-link-&-文档-")
            .tempdir()
            .unwrap();
        let target = temp.path().join("target_🦀");
        let link = temp.path().join("junction_e\u{301}");
        fs::create_dir(&target).unwrap();
        fs::write(temp.path().join("plain.txt"), b"unchanged").unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("midnight-platform/tests/create-junction.ps1");
        let output = Command::new(crate::desktop::powershell_path().unwrap())
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(fixture)
            .arg("-JunctionPath")
            .arg(&link)
            .arg("-TargetPath")
            .arg(&target)
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let cached = fs::read_dir(temp.path())
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path() == link)
            .unwrap()
            .file_type()
            .unwrap();
        assert_eq!(classify(cached), EntryKind::DirectoryLink);
        fs::remove_dir(&target).unwrap();
        assert_eq!(classify(cached), EntryKind::DirectoryLink);
        let mut entries = Vec::new();
        WindowsFileSystem::new(None)
            .read_directory(temp.path(), &Cancellation::default(), &mut |entry| {
                entries.push(entry);
                true
            })
            .unwrap();
        let linked = entries.iter().find(|entry| entry.path == link).unwrap();
        assert_eq!(linked.kind, EntryKind::DirectoryLink);
        assert_eq!(linked.path, link);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "plain.txt")
                .unwrap()
                .kind,
            EntryKind::File
        );
        assert_eq!(
            fs::read(temp.path().join("plain.txt")).unwrap(),
            b"unchanged"
        );
    }
}
