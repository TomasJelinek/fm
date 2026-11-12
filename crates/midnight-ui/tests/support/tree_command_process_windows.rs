//! GetCurrentDirectory reports DOS spelling even when CreateProcess received a verbatim path.
use midnight_core::{Cancellation, TransferService};
use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};

pub fn assert_cwd(actual: &[u8], expected: &Path) {
    assert_eq!(actual.len() % 2, 0);
    let units = actual
        .as_chunks::<2>()
        .0
        .iter()
        .map(|part| u16::from_le_bytes([part[0], part[1]]))
        .collect::<Vec<_>>();
    let path = PathBuf::from(OsString::from_wide(&units));
    let native = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let actual_pin = native.prepare_transfer_destination(&path, &cancel).unwrap();
    let expected_pin = native
        .prepare_transfer_destination(expected, &cancel)
        .unwrap();
    assert_eq!(
        actual_pin.identity(),
        expected_pin.identity(),
        "reported cwd must be the exact native directory"
    );
    let expected = super::raw::bytes(expected.as_os_str());
    let prefix = [92_u8, 0, 92, 0, 63, 0, 92, 0];
    // Only this verified local DOS fixture prefix may differ; all Unicode units remain exact.
    assert!(matches!(expected.get(10..14), Some([58, 0, 92, 0])));
    assert_eq!(actual, expected.strip_prefix(prefix.as_slice()).unwrap());
}
