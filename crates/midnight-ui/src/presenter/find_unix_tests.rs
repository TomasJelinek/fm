use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn find_captures_native_non_utf8_bytes_without_a_display_round_trip() {
    let path = PathBuf::from(OsString::from_vec(b"/native-\xff".to_vec()));
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
