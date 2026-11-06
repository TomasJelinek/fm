//! Native GDI family enumeration; names remain Unicode and never become paths.
use midnight_core::FsError;
use std::{collections::BTreeSet, io, path::Path, ptr};
use windows_sys::Win32::Graphics::Gdi::{
    DEFAULT_CHARSET, EnumFontFamiliesExW, GetDC, LOGFONTW, ReleaseDC, TEXTMETRICW,
};

unsafe extern "system" fn collect(
    font: *const LOGFONTW,
    _: *const TEXTMETRICW,
    _: u32,
    data: isize,
) -> i32 {
    // SAFETY: EnumFontFamiliesExW invokes this synchronously with a live LOGFONTW
    // and the exclusive catalog pointer supplied by families(). Neither escapes.
    let (font, names) = unsafe { (&*font, &mut *(data as *mut BTreeSet<String>)) };
    let length = font
        .lfFaceName
        .iter()
        .position(|&ch| ch == 0)
        .unwrap_or(font.lfFaceName.len());
    if let Ok(name) = String::from_utf16(&font.lfFaceName[..length])
        && !name.starts_with('@')
    {
        names.insert(name);
    }
    i32::from(names.len() < midnight_platform_common::fonts::MAX_FONT_FAMILIES)
}

pub(crate) fn families() -> Result<Vec<String>, FsError> {
    // SAFETY: a null window requests the screen DC. It is released on this same
    // worker thread after the synchronous enumeration has finished.
    let dc = unsafe { GetDC(ptr::null_mut()) };
    if dc.is_null() {
        return Err(FsError::from_io(
            "enumerate fonts",
            Path::new(""),
            io::Error::last_os_error(),
        ));
    }
    let mut names = BTreeSet::<String>::new();
    let mut font: LOGFONTW = unsafe { std::mem::zeroed() };
    font.lfCharSet = DEFAULT_CHARSET;
    // SAFETY: dc is valid, font and names outlive all synchronous callbacks.
    unsafe {
        EnumFontFamiliesExW(
            dc,
            &font,
            Some(collect),
            &mut names as *mut BTreeSet<String> as isize,
            0,
        );
        ReleaseDC(ptr::null_mut(), dc);
    }
    Ok(midnight_platform_common::fonts::normalize_families(names))
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_catalog_contains_complete_valid_unique_family_names() {
        let families = super::families().unwrap();
        assert!(!families.is_empty());
        assert!(families.len() <= midnight_platform_common::fonts::MAX_FONT_FAMILIES);
        assert_eq!(
            families,
            midnight_platform_common::fonts::normalize_families(families.clone())
        );
        for family in families {
            midnight_core::UserPreferences {
                font_family: family,
                ..Default::default()
            }
            .validate()
            .unwrap();
        }
    }
}
