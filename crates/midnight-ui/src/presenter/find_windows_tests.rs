use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn find_captures_an_unpaired_native_surrogate_without_a_display_round_trip() {
    let mut units: Vec<u16> = r"C:\native-".encode_utf16().collect();
    units.push(0xd800);
    let path = PathBuf::from(OsString::from_wide(&units));
    let mut p = Presenter::new(path.clone(), path.clone());
    let session = p.create_find(PaneId::Left).unwrap();
    let requests = std::cell::RefCell::new(Vec::new());
    assert!(
        p.start_find(session, find::FindForm::default(), &|command| {
            if let Command::StartFind(request) = command {
                requests.borrow_mut().push(request);
            }
            true
        })
    );
    assert_eq!(requests.borrow()[0].roots, [path]);
}
