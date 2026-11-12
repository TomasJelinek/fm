use super::*;
pub(super) fn manual(input: &str) -> Result<(Vec<u16>, NodeKind), Failure> {
    let mut units: Vec<_> = input
        .encode_utf16()
        .take(resource::MAX_KEY_BYTES / 2 + 1)
        .collect();
    if units.len() > resource::MAX_KEY_BYTES / 2 {
        return Err(limit());
    }
    if input.contains("://") {
        return Err(unsupported());
    }
    if !units.starts_with(&[92, 92]) {
        if units.contains(&92) {
            return Err(invalid());
        }
        units.splice(..0, [92, 92]);
    }
    let kind = classify(&units)?;
    Ok((units, kind))
}
fn classify(units: &[u16]) -> Result<NodeKind, Failure> {
    let rest = units.strip_prefix(&[92, 92]).ok_or_else(unsupported)?;
    let mut count = 0;
    for part in rest.split(|unit| *unit == 92) {
        if part.is_empty()
            || part == [46]
            || part == [46, 46]
            || part
                .iter()
                .any(|unit| *unit <= 31 || matches!(*unit, 34 | 42 | 47 | 58 | 60 | 62 | 63 | 124))
            || (count == 0 && part.iter().any(|unit| matches!(*unit, 32 | 64)))
        {
            return Err(invalid());
        }
        count += 1;
    }
    Ok(if count == 1 {
        NodeKind::Server
    } else {
        NodeKind::Share
    })
}
pub(super) fn native(units: &[u16]) -> Result<Vec<u8>, Failure> {
    if units.len() > resource::MAX_KEY_BYTES / 2 {
        return Err(limit());
    }
    if classify(units)? != NodeKind::Share {
        return Err(unsupported());
    }
    Ok(units.iter().flat_map(|unit| unit.to_le_bytes()).collect())
}
