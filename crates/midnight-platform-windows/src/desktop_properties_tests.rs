use super::*;
use std::{fs, time::Duration};
use windows::Win32::UI::Shell::{
    IShellItemArray, SHCreateShellItemArrayFromDataObject, SIGDN_FILESYSPATH,
};

fn spelled(path: &str) -> Option<String> {
    shell_spelling(Path::new(path)).map(|path| path.display().to_string())
}

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
        .items
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
fn verbatim_paths_lose_their_prefix_only_where_the_plain_spelling_is_the_same_item() {
    for (verbatim, plain) in [
        (r"\\?\C:\data\文档 é.txt", r"C:\data\文档 é.txt"),
        (r"\\?\d:\Folder\", r"d:\Folder\"),
        (r"\\?\C:\", r"C:\"),
        (
            r"\\?\UNC\server\share\dir\a.txt",
            r"\\server\share\dir\a.txt",
        ),
        (r"\\?\unc\server\share", r"\\server\share"),
        (r"C:\plain\console.txt", r"C:\plain\console.txt"),
        (r"\\server\share\a.txt", r"\\server\share\a.txt"),
    ] {
        assert_eq!(spelled(verbatim).as_deref(), Some(plain), "{verbatim}");
    }
    // Win32 would trim, resolve or redirect these spellings to another item.
    for kept in [
        r"\\?\C:\data\trailing.",
        r"\\?\C:\data\trailing \a.txt",
        r"\\?\C:\data\..\a.txt",
        r"\\?\C:\data\.\a.txt",
        r"\\?\C:\data\\a.txt",
        r"\\?\C:\data\a/b.txt",
        r"\\?\C:\data\NUL",
        r"\\?\C:\data\con.txt",
        r"\\?\C:\data\COM1.log",
        r"\\?\C:\data\lpt²",
        r"\\?\C:\data\CONOUT$",
        r"\\?\UNC\server\share\aux",
    ] {
        assert_eq!(spelled(kept), None, "{kept}");
    }
    // Device and volume namespaces stay verbatim; the Shell then refuses them.
    for other in [
        r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\a.txt",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\a.txt",
    ] {
        assert_eq!(spelled(other).as_deref(), Some(other));
    }
    let names = ["COM", "LPT"].map(|prefix| format!(r"\\?\C:\{prefix}10"));
    for name in names {
        assert!(spelled(&name).is_some(), "{name} is no device");
    }
}

#[test]
fn a_selection_of_one_folder_names_it_and_mixed_folders_name_none() {
    let folder = |paths: &[&str]| {
        let paths: Vec<_> = paths.iter().map(PathBuf::from).collect();
        let selection = property_selection(&paths).unwrap();
        assert_eq!(selection.items.len(), paths.len());
        selection
            .folder
            .map(|folder| String::from_utf16(&folder.0[..folder.0.len() - 1]).unwrap())
    };
    assert_eq!(
        folder(&[r"D:\dir\a.txt", r"\\?\D:\dir\sub", r"D:\dir\c"]).as_deref(),
        Some(r"D:\dir")
    );
    assert_eq!(
        folder(&[r"\\srv\share\a", r"\\?\UNC\srv\share\b"]).as_deref(),
        Some(r"\\srv\share\")
    );
    assert_eq!(folder(&[r"D:\dir\a.txt", r"D:\dir\sub\b.txt"]), None);
    assert_eq!(folder(&[r"D:\a.txt", r"E:\a.txt"]), None);
    // A verbatim item without a plain spelling leaves the selection to the summary.
    let paths = [r"D:\dir\a.txt", r"\\?\D:\dir\b."].map(PathBuf::from);
    assert!(property_selection(&paths).is_none());
}

#[test]
fn verbatim_spellings_beyond_max_path_parse_as_the_same_file_system_items() {
    let fixture = crate::test_fixtures::build_temp("midnight-properties-");
    let folder = fixture.path().join("x".repeat(120)).join("文档".repeat(60));
    fs::create_dir_all(&folder).unwrap();
    let paths = [folder.join("a.txt"), folder.join("b 🧪.txt")];
    for path in &paths {
        fs::write(path, b"data").unwrap();
        assert!(path.as_os_str().len() > 260);
    }
    let verbatim: Vec<_> = paths
        .iter()
        .map(|path| {
            let mut spelled = OsString::from(r"\\?\");
            spelled.push(path);
            PathBuf::from(spelled)
        })
        .collect();
    let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32).unwrap();
    let selection = property_selection(&verbatim).unwrap();
    assert!(selection.folder.is_some());
    let items: Vec<_> = selection
        .items
        .iter()
        .map(|item| PropertyItem::parse(item).expect("a long verbatim path parses"))
        .collect();
    assert_eq!(names(&property_data_object(&items).unwrap()), paths);
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
    assert!(selection.folder.is_none());
    let items: Vec<_> = selection
        .items
        .iter()
        .map(|item| PropertyItem::parse(item).unwrap())
        .collect();
    let data = property_data_object(&items).unwrap();
    assert_eq!(names(&data), paths);
    // The parent of each item's ID list is its folder.
    let parent = PropertyItem::parent_of(&items[2]).unwrap();
    // SAFETY: both PIDLs are live; the Shell only compares them.
    assert!(unsafe { ILIsParent(parent.0.as_ptr(), items[2].0.as_ptr(), true) }.as_bool());
    // SAFETY: as above.
    assert!(!unsafe { ILIsParent(parent.0.as_ptr(), items[0].0.as_ptr(), true) }.as_bool());
}

#[test]
fn one_folder_menu_holds_its_children_and_mixed_folders_have_none() {
    let fixture = crate::test_fixtures::build_temp("midnight-properties-");
    let root = fixture.path();
    let sub = root.join("sub");
    fs::create_dir(&sub).unwrap();
    let mut paths = (0..9)
        .map(|index| root.join(format!("file {index}.txt")))
        .collect::<Vec<_>>();
    for path in &paths {
        fs::write(path, b"data").unwrap();
    }
    paths.push(sub.clone());
    let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32).unwrap();
    let parse = |paths: &[PathBuf]| -> Vec<PropertyItem> {
        property_selection(paths)
            .unwrap()
            .items
            .iter()
            .map(|item| PropertyItem::parse(item).unwrap())
            .collect()
    };
    let items = parse(&paths);
    let menu = folder_menu(ptr::null_mut(), &items).expect("one folder's menu");
    let data: IDataObject = menu_data(&menu);
    assert_eq!(names(&data), paths);
    let nested = sub.join("inner.txt");
    fs::write(&nested, b"x").unwrap();
    assert!(folder_menu(ptr::null_mut(), &parse(&[paths[0].clone(), nested])).is_none());
}

/// The data object a Shell folder's context menu acts on.
fn menu_data(menu: &IContextMenu) -> IDataObject {
    use windows::{Win32::UI::Shell::IObjectWithSelection, core::Interface};
    // The default folder menu exposes the items it acts on.
    let selection: IObjectWithSelection = menu.cast().expect("IObjectWithSelection");
    // SAFETY: the menu is live on this STA.
    unsafe { selection.GetSelection() }.expect("menu selection")
}

/// The file-system paths a Shell data object holds, in order.
fn names(data: &IDataObject) -> Vec<PathBuf> {
    // SAFETY: data and all Shell results are used and released within this STA.
    let items: IShellItemArray = unsafe { SHCreateShellItemArrayFromDataObject(data) }.unwrap();
    // SAFETY: items is a live Shell array on the current STA.
    let count = unsafe { items.GetCount() }.unwrap();
    (0..count)
        .map(|index| {
            // SAFETY: index is in bounds and items is live on the current STA.
            let item = unsafe { items.GetItemAt(index) }.unwrap();
            // SAFETY: item is live on this STA. The allocation is released after copying.
            let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.unwrap();
            // SAFETY: successful GetDisplayName returns a NUL-terminated COM allocation.
            let path = PathBuf::from(OsString::from_wide(unsafe { name.as_wide() }));
            // SAFETY: ownership of this string was transferred by GetDisplayName.
            unsafe { CoTaskMemFree(Some(name.0.cast())) };
            path
        })
        .collect()
}

/// Opens the sheet from a short-lived worker, as the engine's blocking task does, and
/// reads the windows this process shows. Shows native UI, so it runs on request:
/// `cargo test -p midnight-platform-windows --lib several_items -- --ignored`.
#[test]
#[ignore = "opens native property sheets"]
fn several_items_open_one_sheet_whose_items_outlive_the_request() {
    let fixture = crate::test_fixtures::build_temp("midnight-properties-");
    let root = fixture.path();
    let sub = root.join("sub");
    fs::create_dir(&sub).unwrap();
    let files: Vec<_> = (0..9)
        .map(|index| root.join(format!("file {index}.txt")))
        .collect();
    for path in &files {
        fs::write(path, b"12345").unwrap();
    }
    let nested = sub.join("inner.txt");
    fs::write(&nested, b"x").unwrap();
    let mut mixed = files[..2].to_vec();
    mixed.push(nested);
    for (paths, summary, location) in [
        (files.clone(), "9 Files, 0 Folders", "All in "),
        (mixed, "3 Files, 0 Folders", "Various Folders"),
    ] {
        let shown =
            std::thread::spawn(move || WindowsFileSystem::new(None).show_properties(&paths))
                .join()
                .unwrap();
        assert!(shown.unwrap());
        let sheet = native::wait_for(Duration::from_secs(20), |window| {
            window.title == "Desktop" || window.title.ends_with(" Properties")
        })
        .expect("a native window");
        native::close(&sheet);
        assert_ne!(sheet.title, "Desktop", "{sheet:?}");
        assert!(sheet.texts.iter().any(|text| text == summary), "{sheet:?}");
        assert!(
            sheet.texts.iter().any(|text| text.starts_with(location)),
            "{sheet:?}"
        );
        native::wait_closed(&sheet, Duration::from_secs(10));
    }
}

/// This process's visible top-level windows and their controls' text.
mod native {
    use std::time::{Duration, Instant};
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            EnumChildWindows, EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
            IsWindowVisible, PostMessageW, WM_CLOSE,
        },
    };
    use windows_core::BOOL;

    #[derive(Debug)]
    pub(super) struct Window {
        handle: isize,
        pub(super) title: String,
        pub(super) texts: Vec<String>,
    }

    fn text(window: HWND) -> String {
        let mut buffer = [0u16; 1024];
        // SAFETY: the buffer is writable for its length; Win32 validates the handle.
        let length = unsafe { GetWindowTextW(window, &mut buffer) };
        String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap_or(0)])
    }

    unsafe extern "system" fn collect(window: HWND, list: LPARAM) -> BOOL {
        // SAFETY: list is the Vec passed by `windows` below, live through enumeration.
        unsafe { &mut *(list.0 as *mut Vec<HWND>) }.push(window);
        true.into()
    }

    fn windows() -> Vec<Window> {
        let mut all: Vec<HWND> = Vec::new();
        // SAFETY: the callback only appends to this live Vec.
        let _ = unsafe { EnumWindows(Some(collect), LPARAM(&raw mut all as isize)) };
        all.into_iter()
            .filter(|window| {
                let mut process = 0;
                // SAFETY: Win32 validates the handle; process is a valid output.
                unsafe { GetWindowThreadProcessId(*window, Some(&mut process)) };
                // SAFETY: as above.
                process == std::process::id() && unsafe { IsWindowVisible(*window) }.as_bool()
            })
            .map(|window| {
                let mut children: Vec<HWND> = Vec::new();
                // SAFETY: the callback only appends to this live Vec.
                let _ = unsafe {
                    EnumChildWindows(
                        Some(window),
                        Some(collect),
                        LPARAM(&raw mut children as isize),
                    )
                };
                Window {
                    handle: window.0 as isize,
                    title: text(window),
                    texts: children
                        .into_iter()
                        .map(text)
                        .filter(|t| !t.is_empty())
                        .collect(),
                }
            })
            .collect()
    }

    /// The first window `wanted` accepts, once its pages have filled in.
    pub(super) fn wait_for(limit: Duration, wanted: impl Fn(&Window) -> bool) -> Option<Window> {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            if windows().iter().any(&wanted) {
                std::thread::sleep(Duration::from_millis(1500));
                return windows().into_iter().find(|window| wanted(window));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    pub(super) fn close(window: &Window) {
        // SAFETY: a posted WM_CLOSE carries no pointers; Win32 validates the handle.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(window.handle as _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }

    pub(super) fn wait_closed(window: &Window, limit: Duration) {
        let end = Instant::now() + limit;
        // SAFETY: Win32 validates the handle.
        while Instant::now() < end && unsafe { IsWindow(Some(HWND(window.handle as _))) }.as_bool()
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
