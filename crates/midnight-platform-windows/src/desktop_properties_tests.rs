use super::*;
use std::{ffi::OsString, fs, os::windows::ffi::OsStringExt};
use windows::Win32::UI::Shell::{
    IShellItemArray, SHCreateShellItemArrayFromDataObject, SIGDN_FILESYSPATH,
};

#[test]
fn preserves_long_paths_and_exact_utf16_for_every_selected_item() {
    let long = format!("C:\\{}文档👩🏽‍💻.txt", "segment\\".repeat(80));
    let other = [0x44, 0x3a, 0x5c, 0xd800, 0x2e, 0x74, 0x78, 0x74];
    let paths = [
        PathBuf::from(&long),
        PathBuf::from(OsString::from_wide(&other)),
    ];
    let expected = vec![
        long.encode_utf16().chain([0]).collect::<Vec<_>>(),
        other.into_iter().chain([0]).collect(),
    ];
    let actual: Vec<_> = property_selection(&paths)
        .unwrap()
        .into_iter()
        .map(|path| path.0)
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn an_invalid_mark_falls_back_for_the_entire_selection() {
    let filesystem = WindowsFileSystem::new(None);
    for invalid in ["relative.txt", "D:\\bad\0.txt"] {
        let paths = [PathBuf::from("C:\\good.txt"), PathBuf::from(invalid)];
        assert!(!filesystem.show_properties(&paths).unwrap());
    }
    assert!(!filesystem.show_properties(&[]).unwrap());
}

#[test]
fn shell_data_object_carries_each_selected_native_item() {
    let fixture = crate::test_fixtures::build_temp("midnight-properties-");
    let root = fixture.path();
    let folder = root.join("Folder & 文档");
    let paths = [
        root.join("調査 e\u{301}.txt"),
        folder.clone(),
        folder.join("data 🧪.txt"),
    ];
    fs::write(&paths[0], b"first mark").unwrap();
    fs::create_dir(&folder).unwrap();
    fs::write(&paths[2], b"third mark").unwrap();

    let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32).unwrap();
    let selection = property_selection(&paths).unwrap();
    let data = property_data_object(&selection).unwrap();
    // SAFETY: data and all Shell results are used and released within this STA.
    let items: IShellItemArray = unsafe { SHCreateShellItemArrayFromDataObject(&data) }.unwrap();
    // SAFETY: items is a live Shell array on the current STA.
    assert_eq!(unsafe { items.GetCount() }.unwrap(), 3);
    for (index, expected) in paths.iter().enumerate() {
        // SAFETY: index is in bounds and items is live on the current STA.
        let item = unsafe { items.GetItemAt(index as u32) }.unwrap();
        // SAFETY: item is live on this STA. The allocation is released after copying.
        let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.unwrap();
        // SAFETY: successful GetDisplayName returns a NUL-terminated COM allocation.
        let actual = PathBuf::from(OsString::from_wide(unsafe { name.as_wide() }));
        // SAFETY: ownership of this string was transferred by GetDisplayName.
        unsafe { CoTaskMemFree(Some(name.0.cast())) };
        assert_eq!(&actual, expected);
    }
}
