use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn size(value: &str) -> Option<u64> {
    let split = value.bytes().take_while(u8::is_ascii_digit).count();
    if split == 0 {
        return None;
    }
    let number = value[..split].parse::<u64>().ok()?;
    let multiplier = match &value[split..] {
        "" | "B" => 1,
        "kB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "TB" => 1_000_000_000_000,
        "KiB" => 1 << 10,
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        "TiB" => 1 << 40,
        _ => return None,
    };
    number.checked_mul(multiplier)
}

/// Strict Gregorian timestamp with explicit UTC/offset; display/local time is never read.
pub(super) fn timestamp(value: &str) -> Option<i128> {
    if !value.is_ascii() || value.len() < 20 {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let number = |start, end| {
        value
            .get(start..end)?
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| value[start..end].parse::<i64>().ok())
            .flatten()
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year == 0
        || !(1..=12).contains(&month)
        || day < 1
        || day > months[(month - 1) as usize]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut cursor = 19;
    let nanos = if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        let digits = cursor - start;
        if !(1..=9).contains(&digits) {
            return None;
        }
        number(start, cursor)? * 10_i64.pow((9 - digits) as u32)
    } else {
        0
    };
    let offset = match bytes.get(cursor) {
        Some(b'Z') if cursor + 1 == value.len() => 0,
        Some(sign @ (b'+' | b'-')) if cursor + 6 == value.len() && bytes[cursor + 3] == b':' => {
            let hours = number(cursor + 1, cursor + 3)?;
            let minutes = number(cursor + 4, cursor + 6)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            (hours * 3600 + minutes * 60) * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    let preceding = year - 1;
    let days = 365 * preceding + preceding / 4 - preceding / 100
        + preceding / 400
        + months[..(month - 1) as usize].iter().sum::<i64>()
        + day
        - 1
        - 719_162;
    let seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset;
    let total = i128::from(seconds) * 1_000_000_000 + i128::from(nanos);
    Some(total)
}

pub(super) fn instant(value: SystemTime) -> i128 {
    // Duration's u64 seconds plus nanoseconds fit exactly in i128. Do not convert
    // parsed literals through SystemTime: Windows would round sub-100 ns values.
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos())
        }
        Err(error) => {
            let duration = error.duration();
            -(i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos()))
        }
    }
}
