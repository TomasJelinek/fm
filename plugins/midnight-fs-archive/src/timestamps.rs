//! Archive timestamps retain their recorded precision and timezone semantics.
use midnight_filesystem_api::Timestamp;
use std::io::Read;
use zip::extra_fields::ExtraField;

const UNIX_EPOCH_FILETIME_SECONDS: i64 = 11_644_473_600;
const TICKS_PER_SECOND: u64 = 10_000_000;
const NANOS_PER_SECOND: i128 = 1_000_000_000;
const MAX_PAX_TIME_BYTES: usize = 64;
const MAX_PAX_TIME_METADATA: usize = 64 * 1024;

pub(crate) fn filetime(ticks: u64) -> Timestamp {
    // Every u64 FILETIME fits in signed Unix seconds after dividing into100ns ticks.
    Timestamp::Utc {
        seconds: (ticks / TICKS_PER_SECOND) as i64 - UNIX_EPOCH_FILETIME_SECONDS,
        nanoseconds: ((ticks % TICKS_PER_SECOND) * 100) as u32,
    }
}

pub(crate) fn zip_modified<'a>(
    fields: impl Iterator<Item = &'a ExtraField>,
    dos: Option<zip::DateTime>,
) -> Option<Timestamp> {
    let mut precise = None;
    let mut extended = None;
    for field in fields {
        match field {
            ExtraField::Ntfs(value) if precise.is_none() => precise = Some(filetime(value.mtime())),
            ExtraField::ExtendedTimestamp(value) if extended.is_none() => {
                extended = value.mod_time().map(|seconds| Timestamp::Utc {
                    seconds: i64::from(seconds),
                    nanoseconds: 0,
                });
            }
            _ => (),
        }
    }
    precise
        .or(extended)
        .or_else(|| {
            dos.map(|time| Timestamp::Local {
                year: i32::from(time.year()),
                month: time.month(),
                day: time.day(),
                hour: time.hour(),
                minute: time.minute(),
                second: time.second(),
            })
        })
        .filter(|time| time.validate().is_ok())
}

pub(crate) fn tar_modified<R: Read>(entry: &mut ::tar::Entry<'_, R>) -> Option<Timestamp> {
    let fallback = entry
        .header()
        .mtime()
        .ok()
        .and_then(|seconds| i64::try_from(seconds).ok())
        .map(|seconds| Timestamp::Utc {
            seconds,
            nanoseconds: 0,
        });
    let Some(extensions) = entry.pax_extensions().ok()? else {
        return fallback;
    };
    let mut recorded = None;
    let mut bytes = 0usize;
    for extension in extensions {
        let extension = extension.ok()?;
        bytes = bytes
            .checked_add(extension.key_bytes().len())?
            .checked_add(extension.value_bytes().len())?;
        if bytes > MAX_PAX_TIME_METADATA {
            return None;
        }
        if extension.key_bytes() == b"mtime" {
            // Explicit malformed overrides must not turn the placeholder header into a date.
            // PAX records are ordered; a later occurrence overrides an earlier one.
            recorded = Some(pax_time(extension.value_bytes()));
        }
    }
    recorded.unwrap_or(fallback)
}

fn pax_time(value: &[u8]) -> Option<Timestamp> {
    if value.is_empty() || value.len() > MAX_PAX_TIME_BYTES {
        return None;
    }
    let (negative, value) = match value[0] {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => (false, value),
    };
    let (whole, fraction) = match value.iter().position(|byte| *byte == b'.') {
        Some(index) => {
            let fraction = &value[index + 1..];
            if fraction.is_empty() {
                return None;
            }
            (&value[..index], fraction)
        }
        None => (value, &[][..]),
    };
    if whole.is_empty()
        || !whole.iter().all(u8::is_ascii_digit)
        || !fraction.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let seconds = whole.iter().try_fold(0i128, |value, digit| {
        value
            .checked_mul(10)?
            .checked_add(i128::from(*digit - b'0'))
    })?;
    let mut nanos = 0i128;
    for index in 0..9 {
        nanos = nanos * 10
            + fraction
                .get(index)
                .map_or(0, |digit| i128::from(*digit - b'0'));
    }
    // Round subnanosecond precision toward the earlier representable instant.
    if negative
        && fraction
            .get(9..)
            .is_some_and(|tail| tail.iter().any(|digit| *digit != b'0'))
    {
        nanos += 1;
    }
    let magnitude = seconds.checked_mul(NANOS_PER_SECOND)?.checked_add(nanos)?;
    let signed = if negative {
        magnitude.checked_neg()?
    } else {
        magnitude
    };
    Some(Timestamp::Utc {
        seconds: i64::try_from(signed.div_euclid(NANOS_PER_SECOND)).ok()?,
        nanoseconds: signed.rem_euclid(NANOS_PER_SECOND) as u32,
    })
}
