use super::*;
use std::os::windows::ffi::{OsStrExt, OsStringExt};

#[test]
fn unpaired_native_utf16_survives_without_replacement_characters() {
    let value = OsString::from_wide(&[b'f' as u16, 0xd800, b'&' as u16, 0xdcff]);
    let expanded = Template::parse("prefix${file.name}")
        .unwrap()
        .expand(|_| Some(value.as_os_str()))
        .unwrap();
    assert_eq!(
        expanded.encode_wide().collect::<Vec<_>>(),
        vec![
            b'p' as u16,
            b'r' as u16,
            b'e' as u16,
            b'f' as u16,
            b'i' as u16,
            b'x' as u16,
            b'f' as u16,
            0xd800,
            b'&' as u16,
            0xdcff
        ]
    );
}
