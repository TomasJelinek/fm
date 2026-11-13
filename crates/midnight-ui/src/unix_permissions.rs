//! Pure presentation of captured permissions; no filesystem inspection or mutation.
#[cfg(test)]
#[path = "unix_permissions_tests.rs"]
mod tests;

pub(crate) fn matrix_edit(mask: i32, value: i32) -> Result<(u32, u32), String> {
    let mask = u32::try_from(mask).map_err(|_| "Invalid permission mask.")?;
    let value = u32::try_from(value).map_err(|_| "Invalid permission value.")?;
    if mask & !0o7777 != 0 || value & !mask != 0 {
        return Err("Permission edits contain unsupported bits.".into());
    }
    Ok((mask, value))
}

pub(crate) fn parse_octal(text: &str) -> Result<(u32, u32), String> {
    let mode = text.trim();
    if !(3..=4).contains(&mode.len()) || !mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return Err(
            "Permissions require three or four octal digits (for example 755 or 2755).".into(),
        );
    }
    let value = u32::from_str_radix(mode, 8).map_err(|_| "Invalid permissions.")?;
    Ok((if mode.len() == 3 { 0o777 } else { 0o7777 }, value))
}

/// Known positions and their common values, including unchanged special bits.
pub(crate) fn common_modes(modes: impl IntoIterator<Item = Option<u32>>, total: u64) -> (u32, u32) {
    let mut modes = modes.into_iter();
    let Some(Some(first)) = modes.next() else {
        return (0, 0);
    };
    let mut known = 0o7777;
    let mut count = 1;
    for mode in modes {
        let Some(mode) = mode else { return (0, 0) };
        known &= !(first ^ mode);
        count += 1;
    }
    if count != total {
        return (0, 0);
    }
    (known, first & known)
}

pub(crate) struct Projection {
    pub mode: u32,
    pub known: u32,
    pub octal: String,
    pub text: String,
}

pub(crate) fn project(mode: i32, known: i32, mask: i32, value: i32) -> Projection {
    let bits = |value| u32::try_from(value).unwrap_or_default() & 0o7777;
    let (mode, known, mask, value) = (bits(mode), bits(known), bits(mask), bits(value));
    let mode = ((mode & !mask) | (value & mask)) & (known | mask);
    let known = known | mask;
    let mut text = String::with_capacity(9);
    for (read, write, execute, special, low, high) in [
        (0o400, 0o200, 0o100, 0o4000, 's', 'S'),
        (0o040, 0o020, 0o010, 0o2000, 's', 'S'),
        (0o004, 0o002, 0o001, 0o1000, 't', 'T'),
    ] {
        for (bit, letter) in [(read, 'r'), (write, 'w')] {
            text.push(if known & bit == 0 {
                '?'
            } else if mode & bit != 0 {
                letter
            } else {
                '-'
            });
        }
        text.push(if known & (execute | special) != execute | special {
            '?'
        } else if mode & special != 0 {
            if mode & execute != 0 { low } else { high }
        } else if mode & execute != 0 {
            'x'
        } else {
            '-'
        });
    }
    let octal = if known == 0o7777 {
        if mode > 0o777 {
            format!("{mode:04o}")
        } else {
            format!("{mode:03o}")
        }
    } else {
        String::new()
    };
    Projection {
        mode,
        known,
        octal,
        text,
    }
}
