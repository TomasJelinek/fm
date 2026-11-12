//! Owned Unicode child environment. Never mutates the file manager's environment.
use midnight_core::USER_COMMAND_ENVIRONMENT;
use std::{
    cmp::Ordering,
    collections::HashSet,
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    ptr,
};
use windows_sys::Win32::{
    Globalization::CompareStringOrdinal, Storage::FileSystem::GetFullPathNameW,
};

const MAX_UNITS: usize = 4 * 1024 * 1024;

pub(crate) fn snapshot(overrides: &[(String, OsString)]) -> io::Result<Vec<u16>> {
    snapshot_with_inherited(std::env::vars_os().collect(), overrides)
}

pub(crate) fn snapshot_cmd(overrides: &[(String, OsString)]) -> io::Result<Vec<u16>> {
    snapshot_inner(std::env::vars_os().collect(), overrides, true)
}

pub(crate) fn snapshot_with_inherited(
    inherited: Vec<(OsString, OsString)>,
    overrides: &[(String, OsString)],
) -> io::Result<Vec<u16>> {
    snapshot_inner(inherited, overrides, false)
}

fn snapshot_inner(
    mut inherited: Vec<(OsString, OsString)>,
    overrides: &[(String, OsString)],
    cmd_arguments: bool,
) -> io::Result<Vec<u16>> {
    // Native Shell menu initialization can introduce this non-drive private key.
    // Do not serialize it as a custom child variable. Keep malformed values so the
    // strict block encoder still rejects NUL; fixed caller overrides are checked later.
    inherited.retain(|(key, value)| {
        key.as_os_str() != std::ffi::OsStr::new("=::") || value.encode_wide().any(|unit| unit == 0)
    });
    // CreateProcess does not propagate per-drive cwd records with a custom block.
    // GetFullPathNameW("X:") is the documented non-mutating way to retrieve them.
    for letter in b'A'..=b'Z' {
        let key = OsString::from(format!("={}:", letter as char));
        if inherited.iter().any(|(name, _)| *name == key) {
            continue;
        }
        let query = [letter as u16, b':' as u16, 0];
        // SAFETY: terminated fixed Unicode input; null output requests the required size.
        let needed =
            unsafe { GetFullPathNameW(query.as_ptr(), 0, ptr::null_mut(), ptr::null_mut()) };
        if needed == 0 {
            continue;
        }
        if needed as usize > MAX_UNITS {
            return Err(invalid(
                "Inherited drive cwd exceeds the child-environment resource budget.",
            ));
        }
        let mut path = vec![0u16; needed as usize];
        // SAFETY: a live, initialized buffer of exactly the advertised UTF16 capacity.
        let count =
            unsafe { GetFullPathNameW(query.as_ptr(), needed, path.as_mut_ptr(), ptr::null_mut()) };
        if count == 0 || count as usize >= path.len() {
            return Err(io::Error::last_os_error());
        }
        inherited.push((key, OsString::from_wide(&path[..count as usize])));
    }
    if cmd_arguments {
        build_inner(inherited, overrides, true)
    } else {
        build(inherited, overrides)
    }
}

pub(crate) fn build(
    inherited: Vec<(OsString, OsString)>,
    overrides: &[(String, OsString)],
) -> io::Result<Vec<u16>> {
    build_inner(inherited, overrides, false)
}

fn build_inner(
    mut inherited: Vec<(OsString, OsString)>,
    overrides: &[(String, OsString)],
    cmd_arguments: bool,
) -> io::Result<Vec<u16>> {
    let mut keys = HashSet::new();
    for (key, value) in overrides {
        if !USER_COMMAND_ENVIRONMENT.contains(&key.as_str())
            || !keys.insert(key)
            || value.encode_wide().any(|unit| unit == 0)
        {
            return Err(invalid(
                "Custom environment requires unique fixed keys and NUL-free native data.",
            ));
        }
    }
    // Undeclared variables must not inherit stale values from a launching shell.
    inherited.retain(|(key, _)| {
        !key.to_str().is_some_and(|key| {
            USER_COMMAND_ENVIRONMENT.iter().any(|reserved| {
                key.eq_ignore_ascii_case(reserved)
                    || key.eq_ignore_ascii_case(&format!("{reserved}_ARG"))
            })
        })
    });
    inherited.extend(
        overrides
            .iter()
            .map(|(key, value)| (OsString::from(key), value.clone())),
    );
    if cmd_arguments {
        for (key, value) in overrides {
            let mut quoted = Vec::new();
            crate::windows_argv::append_argument(&mut quoted, value, true)?;
            let key = format!("{key}_ARG");
            // Cmd silently ignores oversized inherited variables. Refuse rather
            // than silently losing captured data, including its encoded companion.
            if key.len() + 1 + quoted.len() > 8191 {
                return Err(invalid(
                    "Cmd environment argument exceeds its 8191 UTF16-unit limit. Choose Program or PowerShell.",
                ));
            }
            inherited.push((key.into(), OsString::from_wide(&quoted)));
        }
    }
    let mut total = 1usize;
    let mut records = Vec::with_capacity(inherited.len());
    for (key, value) in inherited {
        let key = key.encode_wide().collect::<Vec<_>>();
        let value = value.encode_wide().collect::<Vec<_>>();
        let drive = key.len() == 3
            && key[0] == b'=' as u16
            && key[1] <= 127
            && (key[1] as u8).is_ascii_alphabetic()
            && key[2] == b':' as u16;
        if key.is_empty()
            || key.contains(&0)
            || value.contains(&0)
            || (!drive && key.contains(&(b'=' as u16)))
        {
            return Err(invalid("Invalid inherited native environment record."));
        }
        total = total
            .saturating_add(key.len())
            .saturating_add(value.len())
            .saturating_add(2);
        if total > MAX_UNITS {
            return Err(invalid(
                "Child environment exceeds its 8 MiB resource budget.",
            ));
        }
        records.push((key, value));
    }
    let mut comparison_failed = false;
    records.sort_by(|(left, _), (right, _)| match compare(left, right) {
        Some(order) => order,
        None => {
            comparison_failed = true;
            Ordering::Equal
        }
    });
    if comparison_failed {
        return Err(io::Error::last_os_error());
    }
    if records
        .windows(2)
        .any(|pair| compare(&pair[0].0, &pair[1].0) == Some(Ordering::Equal))
    {
        return Err(invalid(
            "Inherited environment contains ambiguous case-insensitive duplicate names.",
        ));
    }
    let mut block = Vec::with_capacity(total.max(2));
    for (name, value) in records {
        block.extend(name);
        block.push(b'=' as u16);
        block.extend(value);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}
fn compare(left: &[u16], right: &[u16]) -> Option<Ordering> {
    // SAFETY: bounded UTF16 buffers with explicit lengths; ordinal case folding is locale-free.
    match unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        )
    } {
        1 => Some(Ordering::Less),
        2 => Some(Ordering::Equal),
        3 => Some(Ordering::Greater),
        _ => None,
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
