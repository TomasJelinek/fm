use crate::*;
use std::borrow::Cow;

/// Names remain raw provider data. This predicate only permits byte-stream access;
/// it never turns a member into a native path or silently repairs its spelling.
/// `.` components (`./usr/bin/tool`) name the current directory and are harmless,
/// but a regular file cannot be named by a final `.`.
pub(crate) fn safe_name(name: &[u8]) -> bool {
    safe_path(name) && name.rsplit(|b| *b == b'/').next() != Some(b".")
}

/// A safe directory record may end in `.` (`./`, `a/.`): it names that directory.
fn safe_path(name: &[u8]) -> bool {
    !name.is_empty()
        && !name.contains(&0)
        && !name.contains(&b'\\')
        && !name.contains(&b':')
        && !name.starts_with(b"/")
        && name
            .split(|b| *b == b'/')
            .all(|part| !part.is_empty() && part != b"..")
}

impl Archive {
    /// Add the bounded declared length to the just-indexed regular file.
    pub(crate) fn indexed_size(&mut self, size: u64) -> Result<u64, Error> {
        self.total = self.total.checked_add(size).ok_or_else(Error::limit)?;
        if size > self.limits.max_entry || self.total > self.limits.max_total {
            return Err(Error::limit());
        }
        let node = self.nodes.last_mut().ok_or_else(Error::limit)?;
        node.size = Some(size);
        Ok(node.id)
    }
    /// Index one member. Returns its node ID; a `./` directory record names the
    /// root (ID 1), which has no node or metadata of its own.
    pub(crate) fn add(
        &mut self,
        raw: &[u8],
        display: &str,
        kind: Kind,
        data: Option<Content>,
        readable: bool,
    ) -> Result<u64, Error> {
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
        let safe = if kind == Kind::Directory {
            safe_path(raw)
        } else {
            safe_name(raw)
        };
        if !safe {
            // Unsafe records stay visible at the root under their complete raw name.
            // They never become navigable, always-empty directories named `..` or `C:`.
            let kind = if kind == Kind::Directory {
                Kind::Other
            } else {
                kind
            };
            return self.push(1, raw, display, kind, data, false);
        }
        let parts: Vec<&[u8]> = raw.split(|b| *b == b'/').collect();
        let labels: Vec<&str> = display.trim_end_matches('/').split('/').collect();
        let matching = parts.len() == labels.len();
        let last = parts.len() - 1;
        let components: Vec<(&[u8], Cow<'_, str>)> = parts
            .iter()
            .enumerate()
            .filter(|(_, part)| **part != b".")
            .map(|(index, part)| {
                let label = if matching {
                    Cow::Borrowed(labels[index])
                } else if index == last {
                    Cow::Borrowed(labels[labels.len() - 1])
                } else {
                    String::from_utf8_lossy(part)
                };
                (*part, label)
            })
            .collect();
        let Some(((name, label), parents)) = components.split_last() else {
            return Ok(1);
        };
        let mut parent = 1;
        for (part, label) in parents {
            parent = self.directory(parent, part, label)?;
        }
        if kind == Kind::Directory {
            return self.directory(parent, name, label);
        }
        self.push(
            parent,
            name,
            label,
            kind,
            data,
            readable && kind == Kind::File,
        )
    }
    /// The directory `name` below `parent`, created on first encounter.
    fn directory(&mut self, parent: u64, name: &[u8], label: &str) -> Result<u64, Error> {
        let key = (parent, name.to_vec());
        if let Some(existing) = self.directories.get(&key) {
            return Ok(*existing);
        }
        let id = self.push(parent, name, label, Kind::Directory, None, false)?;
        self.directories.insert(key, id);
        Ok(id)
    }
    fn push(
        &mut self,
        parent: u64,
        name: &[u8],
        display: &str,
        kind: Kind,
        data: Option<Content>,
        readable: bool,
    ) -> Result<u64, Error> {
        self.names = self
            .names
            .checked_add(name.len() + display.len())
            .ok_or_else(Error::limit)?;
        if self.nodes.len() >= self.limits.max_entries || self.names > self.limits.max_names {
            return Err(Error::limit());
        }
        let size = data.as_ref().map(|v| v.length);
        self.total = self
            .total
            .checked_add(size.unwrap_or(0))
            .ok_or_else(Error::limit)?;
        if size.unwrap_or(0) > self.limits.max_entry || self.total > self.limits.max_total {
            return Err(Error::limit());
        }
        let id = self.nodes.len() as u64 + 2;
        self.children
            .entry(parent)
            .or_default()
            .push(self.nodes.len());
        self.nodes.push(Node {
            id,
            parent,
            name: name.to_vec(),
            display: display.into(),
            kind,
            size,
            readable,
            modified: None,
        });
        if let Some(data) = data {
            self.contents.insert(id, data);
        }
        Ok(id)
    }
}
