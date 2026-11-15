use crate::{
    adapter::NodeKind,
    api::{ErrorCode, Failure},
};

pub(super) const MAX_URI_BYTES: usize = 64 * 1024;
fn invalid() -> Failure {
    Failure::new(ErrorCode::Invalid, "Invalid network location.")
}

fn decoded(value: &str) -> Result<Vec<u8>, Failure> {
    let mut output = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        let byte = if byte == b'%' {
            let high = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(invalid)?;
            let low = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(invalid)?;
            (high * 16 + low) as u8
        } else {
            byte
        };
        if byte.is_ascii_control() || byte == b'\\' {
            return Err(invalid());
        }
        output.push(byte);
    }
    Ok(output)
}

pub(super) fn classify(key: &[u8]) -> Result<NodeKind, Failure> {
    if key.len() > MAX_URI_BYTES {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "Network address exceeds the admission budget.",
        ));
    }
    let uri = std::str::from_utf8(key).map_err(|_| invalid())?;
    if uri.bytes().any(|b| b.is_ascii_control() || b == b'\\') || uri.contains(['?', '#']) {
        return Err(invalid());
    }
    let (scheme, rest) = uri.split_once("://").ok_or_else(invalid)?;
    if !matches!(scheme, "network" | "smb" | "dns-sd" | "wsdd") {
        return Err(invalid());
    }
    if rest.is_empty() {
        return Err(invalid());
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = decoded(authority)?;
    if host.contains(&b'@') || host.contains(&b'/') || host.contains(&b'%') || host.contains(&b' ')
    {
        return Err(invalid());
    }
    // A colon is permitted only for a numeric port or a bracketed IPv6 address.
    let authority_text = std::str::from_utf8(&host).map_err(|_| invalid())?;
    if let Some(ipv6) = authority_text.strip_prefix('[') {
        let (ip, suffix) = ipv6.split_once(']').ok_or_else(invalid)?;
        if ip.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(invalid());
        }
        if !suffix.is_empty() && !valid_port(suffix.strip_prefix(':').ok_or_else(invalid)?) {
            return Err(invalid());
        }
    } else if let Some((hostname, port)) = authority_text.split_once(':')
        && (hostname.is_empty() || !valid_port(port))
    {
        return Err(invalid());
    }
    let mut components = 0;
    for component in path.split('/') {
        if component.is_empty() {
            continue;
        }
        let component = decoded(component)?;
        if component == b"." || component == b".." || component.contains(&b'/') {
            return Err(invalid());
        }
        components += 1;
    }
    match scheme {
        "network" if authority.is_empty() && path.is_empty() => Ok(NodeKind::Root),
        "wsdd" if uri == "wsdd:///" => Ok(NodeKind::Container),
        "dns-sd" if !authority.is_empty() => Ok(NodeKind::Container),
        "smb" if authority.is_empty() && path.is_empty() => Ok(NodeKind::Container),
        "smb" if !authority.is_empty() && components == 0 => Ok(NodeKind::Server),
        "smb" if !authority.is_empty() && components == 1 => Ok(NodeKind::Share),
        _ => Err(invalid()),
    }
}
fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|p| p != 0)
}

pub(super) fn manual(input: &str) -> Result<(Vec<u8>, NodeKind), Failure> {
    if input.len() > MAX_URI_BYTES {
        return Err(Failure::new(
            ErrorCode::ResourceLimit,
            "Network address exceeds the admission budget.",
        ));
    }
    let key = if input.contains("://") {
        input.as_bytes().to_vec()
    } else {
        if input.is_empty()
            || input.contains(['/', '\\', '@', ':', '%', '?', '#'])
            || input.chars().any(char::is_whitespace)
        {
            return Err(invalid());
        }
        format!("smb://{input}/").into_bytes()
    };
    let kind = classify(&key)?;
    Ok((key, kind))
}
