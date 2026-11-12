use super::*;

#[test]
fn native_activation_is_revalidated_when_transport_bypasses_wire_codec() {
    for path in [
        r"\\.\pipe\name",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1",
        r"\\server\share\..\other",
        r"\\?\UNC\?\share",
        r"\\server\share\file:stream",
    ] {
        let (provider, fake) = fixture(vec![
            resolved(),
            Response::Activated(ActivationTarget::Native(Address {
                encoding: AddressEncoding::WindowsUtf16Le,
                bytes: path.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            })),
        ]);
        assert!(
            provider.activate_inner(&address(), &|| false).is_err(),
            "{path}"
        );
        assert_eq!(
            fake.calls.lock().unwrap().last(),
            Some(&Request::CloseMount { mount: 1 })
        );
    }
}

#[test]
fn native_activation_preserves_long_unc_paths_and_raw_utf16() {
    use std::os::windows::ffi::OsStrExt;
    let mut units: Vec<_> = format!("\\\\server\\share\\{}", "文档\\".repeat(300))
        .encode_utf16()
        .collect();
    units.push(0xd800);
    let (provider, _) = fixture(vec![
        resolved(),
        Response::Activated(ActivationTarget::Native(Address {
            encoding: AddressEncoding::WindowsUtf16Le,
            bytes: units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
        })),
    ]);
    let NetworkDestination::Native(path) = provider.activate_inner(&address(), &|| false).unwrap()
    else {
        panic!()
    };
    assert_eq!(path.as_os_str().encode_wide().collect::<Vec<_>>(), units);
}
