//! Windows address grammar is validated on every host, without interpreting or
//! normalizing UTF-16. Only filesystem drive/UNC prefixes qualify for activation.

pub(super) fn absolute(units: &[u16]) -> bool {
    if let Some(rest) = units.strip_prefix(&[92, 92, 63, 92]) {
        if rest.contains(&47) {
            return false;
        }
        // Verbatim drive and UNC paths retain long/raw names. Other verbatim
        // namespaces (GLOBALROOT, Volume GUID, pipes/devices) are not handoffs.
        if rest.get(..4).is_some_and(|prefix| {
            matches!(prefix[0], 85 | 117)
                && matches!(prefix[1], 78 | 110)
                && matches!(prefix[2], 67 | 99)
                && prefix[3] == 92
        }) {
            return unc(&rest[4..]);
        }
        return drive(rest);
    }
    units
        .strip_prefix(&[92, 92])
        .map_or_else(|| drive(units), unc)
}

fn separator(unit: &u16) -> bool {
    matches!(unit, 47 | 92)
}

fn component(units: &[u16]) -> bool {
    !units.is_empty()
        && units != [46]
        && units != [46, 46]
        && units
            .iter()
            .all(|unit| !matches!(unit, 0..=31 | 34 | 42 | 58 | 60 | 62 | 63 | 124))
}

fn descendants(units: &[u16]) -> bool {
    if units.is_empty() {
        return true;
    }
    // A single final separator is valid; empty interior components are not.
    let units = units
        .strip_suffix(&[92])
        .or_else(|| units.strip_suffix(&[47]))
        .unwrap_or(units);
    !units.is_empty() && units.split(separator).all(component)
}

fn drive(units: &[u16]) -> bool {
    units.len() >= 3
        && matches!(units[0], 65..=90 | 97..=122)
        && units[1] == 58
        && separator(&units[2])
        && descendants(&units[3..])
}

fn unc(units: &[u16]) -> bool {
    let mut parts = units.splitn(3, separator);
    parts.next().is_some_and(component)
        && parts.next().is_some_and(component)
        && parts.next().is_none_or(descendants)
}
