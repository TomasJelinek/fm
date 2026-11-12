use crate::{CpuBudget, Limits, ResourcePolicy};
use std::io::{self, Read, Write};

pub(crate) fn read(reader: &mut impl Read) -> io::Result<(ResourcePolicy, &'static [u8; 4])> {
    let mut magic = [0; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"MWS1" && &magic != b"MWS2" {
        return Err(invalid());
    }
    let mut memory = [0; 8];
    reader.read_exact(&mut memory)?;
    let memory_bytes = u64::from_le_bytes(memory);
    if &magic == b"MWS1" {
        let limits = Limits {
            memory_bytes,
            cpu_seconds: seconds(reader)?,
        }
        .validate()?;
        return Ok((limits.into(), b"MWR1"));
    }
    let mut tag = [0];
    reader.read_exact(&mut tag)?;
    let cpu = match tag[0] {
        0 => CpuBudget::NoAdditionalLimit,
        1 => CpuBudget::Seconds(seconds(reader)?),
        _ => return Err(invalid()),
    };
    Ok((ResourcePolicy { memory_bytes, cpu }.validate()?, b"MWR2"))
}

/// Explicit version2 startup. Legacy `write_limits` continues to emit MWS1.
pub fn write_policy(writer: &mut impl Write, policy: ResourcePolicy) -> io::Result<()> {
    let policy = policy.validate()?;
    writer.write_all(b"MWS2")?;
    writer.write_all(&policy.memory_bytes.to_le_bytes())?;
    match policy.cpu {
        CpuBudget::NoAdditionalLimit => writer.write_all(&[0])?,
        CpuBudget::Seconds(seconds) => {
            writer.write_all(&[1])?;
            writer.write_all(&seconds.to_le_bytes())?;
        }
    }
    writer.flush()
}

/// Accept only a version2 sandbox acknowledgement for an explicit resource policy.
pub fn read_policy_ready(reader: &mut impl Read) -> io::Result<()> {
    let mut ready = [0; 4];
    reader.read_exact(&mut ready)?;
    if &ready != b"MWR2" {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Worker policy sandbox unavailable",
        ));
    }
    Ok(())
}

fn seconds(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid worker startup")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_decode_exactly_and_leave_provider_bytes_untouched() {
        for cpu in [CpuBudget::NoAdditionalLimit, CpuBudget::Seconds(3)] {
            let policy = ResourcePolicy {
                memory_bytes: 64 * 1024 * 1024,
                cpu,
            };
            let mut bytes = Vec::new();
            write_policy(&mut bytes, policy).unwrap();
            let length = bytes.len();
            bytes.extend(b"provider");
            let mut reader = io::Cursor::new(&bytes);
            assert_eq!(read(&mut reader).unwrap(), (policy, b"MWR2"));
            assert_eq!(reader.position(), length as u64);
            for end in 0..length {
                assert!(read(&mut &bytes[..end]).is_err());
            }
        }
        let limits = Limits {
            memory_bytes: 64 * 1024 * 1024,
            cpu_seconds: 15,
        };
        let mut bytes = Vec::new();
        crate::write_limits(&mut bytes, limits).unwrap();
        assert_eq!(
            read(&mut bytes.as_slice()).unwrap(),
            (limits.into(), b"MWR1")
        );
        bytes[12..16].fill(0);
        assert!(read(&mut bytes.as_slice()).is_err());
    }
    #[test]
    fn invalid_version_tags_memory_and_zero_limited_cpu_fail_closed() {
        let policy = ResourcePolicy {
            memory_bytes: 64 * 1024 * 1024,
            cpu: CpuBudget::Seconds(3),
        };
        let mut encoded = Vec::new();
        write_policy(&mut encoded, policy).unwrap();
        for (offset, value) in [(3, b'3'), (12, 2), (7, 0), (13, 0)] {
            let mut bytes = encoded.clone();
            bytes[offset] = value;
            assert!(read(&mut bytes.as_slice()).is_err(), "{offset}");
        }
    }
}
