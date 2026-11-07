use crate::*;

/// Names remain raw provider data. This predicate only permits byte-stream access;
/// it never turns a member into a native path or silently repairs its spelling.
pub(crate) fn safe_name(name: &[u8]) -> bool {
    !name.is_empty()
        && !name.contains(&0)
        && !name.contains(&b'\\')
        && !name.contains(&b':')
        && !name.starts_with(b"/")
        && name
            .split(|b| *b == b'/')
            .all(|part| !part.is_empty() && part != b"." && part != b"..")
}

impl Archive {
    pub(crate) fn add(
        &mut self,
        raw: &[u8],
        display: &str,
        kind: Kind,
        data: Option<Vec<u8>>,
        readable: bool,
    ) -> Result<(), Error> {
        let raw = if kind == Kind::Directory {
            raw.strip_suffix(b"/").unwrap_or(raw)
        } else {
            raw
        };
        if raw.len() > midnight_filesystem_api::MAX_NAME
            || display.len() > midnight_filesystem_api::MAX_NAME
        {
            return Err(Error::limit());
        }
        let safe = safe_name(raw);
        let mut parent = 1;
        let mut parts = raw.split(|b| *b == b'/').peekable();
        if safe {
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    break;
                }
                let key = (parent, part.to_vec());
                if let Some(existing) = self.directories.get(&key) {
                    parent = *existing;
                } else {
                    parent = self.push(
                        parent,
                        part,
                        &String::from_utf8_lossy(part),
                        Kind::Directory,
                        None,
                        false,
                    )?;
                    self.directories.insert(key, parent);
                }
            }
        }
        let name = if safe {
            raw.rsplit(|b| *b == b'/').next().unwrap_or(raw)
        } else {
            raw
        };
        let display = if safe {
            display
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(display)
        } else {
            display
        };
        if safe
            && kind == Kind::Directory
            && self.directories.contains_key(&(parent, name.to_vec()))
        {
            return Ok(());
        }
        let id = self.push(
            parent,
            name,
            display,
            kind,
            data,
            readable && safe && kind == Kind::File,
        )?;
        if safe && kind == Kind::Directory {
            self.directories.insert((parent, name.to_vec()), id);
        }
        Ok(())
    }
    fn push(
        &mut self,
        parent: u64,
        name: &[u8],
        display: &str,
        kind: Kind,
        data: Option<Vec<u8>>,
        readable: bool,
    ) -> Result<u64, Error> {
        self.names = self
            .names
            .checked_add(name.len() + display.len())
            .ok_or_else(Error::limit)?;
        if self.nodes.len() >= self.limits.max_entries || self.names > self.limits.max_names {
            return Err(Error::limit());
        }
        let size = data.as_ref().map(|v| v.len() as u64);
        self.total = self
            .total
            .checked_add(size.unwrap_or(0))
            .ok_or_else(Error::limit)?;
        if size.unwrap_or(0) > self.limits.max_entry || self.total > self.limits.max_total {
            return Err(Error::limit());
        }
        let id = self.nodes.len() as u64 + 2;
        self.nodes.push(Node {
            id,
            parent,
            name: name.to_vec(),
            display: display.into(),
            kind,
            size,
            readable,
        });
        if let Some(data) = data {
            self.contents.insert(id, data);
        }
        Ok(id)
    }
}
