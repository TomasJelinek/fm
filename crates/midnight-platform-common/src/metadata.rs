//! Unicode transformations of one native filename component. No normalization or case folding.
use midnight_core::{
    CaseChange, CaseMode, CaseScope, FsError, MetadataCapabilities, MetadataPatch, TimeEdit,
};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

pub fn case_name(name: &OsStr, change: CaseChange) -> Result<OsString, FsError> {
    let units = native::units(name);
    let dot = units
        .iter()
        .rposition(|&unit| unit == 46)
        .filter(|&index| index > 0);
    let range = match change.scope {
        CaseScope::Whole => 0..units.len(),
        CaseScope::Basename => 0..dot.unwrap_or(units.len()),
        CaseScope::Extension => dot.map_or(units.len()..units.len(), |i| i + 1..units.len()),
    };
    if range.is_empty() {
        return Ok(name.to_owned());
    }
    let text = native::decode(&units[range.clone()]).ok_or_else(|| {
        FsError::from_io(
            "change filename case",
            Path::new(name),
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "the selected filename component is not valid Unicode",
            ),
        )
    })?;
    let mode = match (change.mode, change.scope) {
        (CaseMode::MixedNameLowerExtension, CaseScope::Basename) => CaseMode::Mixed,
        (CaseMode::MixedNameLowerExtension, CaseScope::Extension) => CaseMode::Lower,
        (mode, _) => mode,
    };
    let transformed = match mode {
        CaseMode::Lower => lower(&text),
        CaseMode::Upper => icu_casemap::CaseMapper::new()
            .uppercase_to_string(&text, &icu_locale_core::langid!("und"))
            .into_owned(),
        CaseMode::Mixed => mixed(&text, true),
        CaseMode::MixedNameLowerExtension => {
            let split = text
                .rfind('.')
                .filter(|&index| index > 0)
                .unwrap_or(text.len());
            format!("{}{}", mixed(&text[..split], false), lower(&text[split..]))
        }
    };
    let mut result = units[..range.start].to_vec();
    result.extend(native::units(OsStr::new(&transformed)));
    result.extend_from_slice(&units[range.end..]);
    Ok(native::name(&result))
}

fn lower(text: &str) -> String {
    icu_casemap::CaseMapper::new()
        .lowercase_to_string(text, &icu_locale_core::langid!("und"))
        .into_owned()
}

/// Salamander-compatible boundaries: spaces, plus dots in fully mixed mode.
/// Unicode titlecase (e.g. ß -> Ss) is distinct from uppercase (ß -> SS).
fn mixed(text: &str, dots: bool) -> String {
    use icu_casemap::{
        CaseMapper,
        options::{LeadingAdjustment, TitlecaseOptions},
    };
    let mapper = CaseMapper::new();
    let root = icu_locale_core::langid!("und");
    let mut options = TitlecaseOptions::default();
    options.leading_adjustment = Some(LeadingAdjustment::None);
    let mut result = String::new();
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        if ch == ' ' || (dots && ch == '.') {
            result.push_str(&mapper.titlecase_segment_with_only_case_data_to_string(
                &text[start..index],
                &root,
                options,
            ));
            result.push(ch);
            start = index + ch.len_utf8();
        }
    }
    result.push_str(&mapper.titlecase_segment_with_only_case_data_to_string(
        &text[start..],
        &root,
        options,
    ));
    result
}

#[cfg(windows)]
#[path = "metadata/native_windows.rs"]
mod native;
#[cfg(unix)]
#[path = "metadata/native_unix.rs"]
mod native;

pub fn validate_patch(
    patch: &MetadataPatch,
    caps: MetadataCapabilities,
    path: &Path,
) -> Result<(), FsError> {
    let invalid = patch.windows_value & !patch.windows_mask != 0
        || patch.unix_value & !patch.unix_mask != 0
        || patch.windows_mask & !0x27 != 0
        || patch.unix_mask & !0o7777 != 0
        || [patch.created, patch.modified, patch.accessed].contains(&TimeEdit::Now);
    let unsupported = patch.windows_mask & !caps.windows_attributes != 0
        || patch.unix_mask & !caps.unix_permissions != 0
        || (patch.compressed.is_some() && !caps.compressed)
        || (patch.created != TimeEdit::Keep && !caps.created)
        || (patch.modified != TimeEdit::Keep && !caps.modified)
        || (patch.accessed != TimeEdit::Keep && !caps.accessed)
        || (patch.case.is_some() && !caps.rename);
    if invalid || unsupported {
        return Err(FsError::from_io(
            "validate metadata edit",
            path,
            std::io::Error::new(
                if invalid {
                    std::io::ErrorKind::InvalidInput
                } else {
                    std::io::ErrorKind::Unsupported
                },
                if invalid {
                    "invalid metadata mask or unresolved timestamp"
                } else {
                    "one or more requested fields cannot be changed on this entry"
                },
            ),
        ));
    }
    Ok(())
}
