use super::*;
use std::cell::Cell;
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_CANCELLED};

#[test]
fn connect_and_disconnect_dispatch_only_the_requested_native_dialog() {
    let called = Cell::new(0);
    dispatch(
        false,
        || {
            called.set(1);
            0
        },
        || panic!("disconnect must not be called"),
    )
    .unwrap();
    assert_eq!(called.get(), 1);
    dispatch(
        true,
        || panic!("connect must not be called"),
        || {
            called.set(2);
            0
        },
    )
    .unwrap();
    assert_eq!(called.get(), 2);
}

#[test]
fn native_cancellation_is_success_and_access_denial_remains_an_error() {
    for code in [u32::MAX, ERROR_CANCELLED] {
        dispatch(false, || code, || unreachable!()).unwrap();
    }
    assert_eq!(
        dispatch(false, || ERROR_ACCESS_DENIED, || unreachable!())
            .unwrap_err()
            .kind,
        io::ErrorKind::PermissionDenied
    );
}
