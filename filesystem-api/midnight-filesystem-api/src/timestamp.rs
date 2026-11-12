use crate::{invalid, wire};
use std::io::{self, Read};

/// Raw archive time. A local civil timestamp does not imply a UTC offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timestamp {
    Utc {
        seconds: i64,
        nanoseconds: u32,
    },
    Local {
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    },
}

impl Timestamp {
    pub fn validate(self) -> io::Result<()> {
        let valid = match self {
            Self::Utc { nanoseconds, .. } => nanoseconds < 1_000_000_000,
            Self::Local {
                year,
                month,
                day,
                hour,
                minute,
                second,
            } => {
                let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                let days = match month {
                    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                    4 | 6 | 9 | 11 => 30,
                    2 if leap => 29,
                    2 => 28,
                    _ => 0,
                };
                (1..=9999).contains(&year)
                    && day > 0
                    && day <= days
                    && hour < 24
                    && minute < 60
                    && second < 60
            }
        };
        if valid { Ok(()) } else { Err(invalid()) }
    }
}

pub(crate) fn write(out: &mut Vec<u8>, value: Option<Timestamp>) -> io::Result<()> {
    if let Some(value) = value {
        value.validate()?;
    }
    match value {
        None => out.push(0),
        Some(Timestamp::Utc {
            seconds,
            nanoseconds,
        }) => {
            out.push(1);
            out.extend(seconds.to_le_bytes());
            out.extend(nanoseconds.to_le_bytes());
        }
        Some(Timestamp::Local {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }) => {
            out.push(2);
            out.extend(year.to_le_bytes());
            out.extend([month, day, hour, minute, second]);
        }
    }
    Ok(())
}

pub(crate) fn read(input: &mut impl Read) -> io::Result<Option<Timestamp>> {
    let value = match wire::byte(input)? {
        0 => return Ok(None),
        1 => {
            let mut seconds = [0; 8];
            let mut nanoseconds = [0; 4];
            input.read_exact(&mut seconds)?;
            input.read_exact(&mut nanoseconds)?;
            Timestamp::Utc {
                seconds: i64::from_le_bytes(seconds),
                nanoseconds: u32::from_le_bytes(nanoseconds),
            }
        }
        2 => {
            let mut year = [0; 4];
            input.read_exact(&mut year)?;
            Timestamp::Local {
                year: i32::from_le_bytes(year),
                month: wire::byte(input)?,
                day: wire::byte(input)?,
                hour: wire::byte(input)?,
                minute: wire::byte(input)?,
                second: wire::byte(input)?,
            }
        }
        _ => return Err(invalid()),
    };
    value.validate()?;
    Ok(Some(value))
}
