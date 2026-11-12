//! Windows C-runtime argument encoding, shared by process argv and Cmd companions.
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt};

pub(crate) const MAX_PROCESS_UTF16: usize = 32767;

pub(crate) fn wide_units(value: &OsStr) -> io::Result<Vec<u16>> {
    let units: Vec<_> = value.encode_wide().take(MAX_PROCESS_UTF16 + 1).collect();
    if units.contains(&0) || units.len() >= MAX_PROCESS_UTF16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows process arguments must fit native UTF16 limits and contain no NUL",
        ));
    }
    Ok(units)
}

/// Windows C-runtime quoting for ordinary argv; raw Cmd tails bypass this function.
pub(crate) fn append_argument(
    line: &mut Vec<u16>,
    argument: &OsStr,
    force_quotes: bool,
) -> io::Result<()> {
    let units = wide_units(argument)?;
    if !force_quotes
        && !units.is_empty()
        && !units
            .iter()
            .any(|unit| [b' ' as u16, b'\t' as u16, b'"' as u16].contains(unit))
    {
        line.extend(units);
        return Ok(());
    }
    line.push(b'"' as u16);
    let mut slashes = 0usize;
    for unit in units {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            line.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        line.push(unit);
        slashes = 0;
    }
    line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    line.push(b'"' as u16);
    Ok(())
}
