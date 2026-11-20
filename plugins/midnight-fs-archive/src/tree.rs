use crate::*;
use midnight_filesystem_api::Timestamp;
use std::borrow::Cow;

/// Display length of a member whose label is its raw name read as UTF-8.
const SAME_DISPLAY: u32 = u32::MAX;

/// One indexed member; its ID is its position plus 2 (the root, ID 1, has none). Its
/// raw name and a label that differs from it lie in the archive's name arena, so a
/// member costs this record and its name bytes instead of a public `Node` with two
/// allocations of its own: indexing a million TAR members fell from 241 to 105 bytes
/// each at peak (`measure_index_memory` in tests/performance.rs).
pub(crate) struct Member {
    pub(crate) parent: u64,
    /// Start of the raw name in the arena; a label that differs from it follows it.
    name: usize,
    name_length: u32,
    /// `SAME_DISPLAY`, or the length of the label stored after the raw name.
    display_length: u32,
    size: u64,
    pub(crate) modified: Option<Timestamp>,
    pub(crate) kind: Kind,
    pub(crate) readable: bool,
    sized: bool,
}
impl Member {
    pub(crate) fn size(&self) -> Option<u64> {
        self.sized.then_some(self.size)
    }
}

/// Look up `id` in (ID, value) pairs pushed in increasing ID order, as members are
/// indexed: a sorted list costs a fraction of a hash map per member.
pub(crate) fn find<T>(pairs: &[(u64, T)], id: u64) -> Option<&T> {
    let at = pairs.binary_search_by_key(&id, |(key, _)| *key).ok()?;
    Some(&pairs[at].1)
}

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
    pub(crate) fn member(&self, id: u64) -> Option<&Member> {
        self.members.get(usize::try_from(id.checked_sub(2)?).ok()?)
    }

    pub(crate) fn member_mut(&mut self, id: u64) -> Option<&mut Member> {
        self.members
            .get_mut(usize::try_from(id.checked_sub(2)?).ok()?)
    }

    /// The raw name and the label of the member at `position`.
    fn names_of(&self, position: usize) -> (&[u8], Cow<'_, str>) {
        let member = &self.members[position];
        let end = member.name + member.name_length as usize;
        let name = &self.names[member.name..end];
        let display = match member.display_length {
            // Stored only from a `&str` whose bytes equal the raw name.
            SAME_DISPLAY => name,
            length => &self.names[end..end + length as usize],
        };
        (name, String::from_utf8_lossy(display))
    }

    /// Bytes of both names of the member at `position`, as a listing frame holds them.
    pub(crate) fn name_bytes(&self, position: usize) -> usize {
        let member = &self.members[position];
        let name = member.name_length as usize;
        name + match member.display_length {
            SAME_DISPLAY => name,
            length => length as usize,
        }
    }

    /// The public node of the member at `position`.
    pub(crate) fn node(&self, position: usize) -> Node {
        let member = &self.members[position];
        let (name, display) = self.names_of(position);
        Node {
            id: position as u64 + 2,
            parent: member.parent,
            name: name.to_vec(),
            display: display.into_owned(),
            kind: member.kind,
            size: member.size(),
            readable: member.readable,
            modified: member.modified,
        }
    }

    /// Release what only indexing needed and the spare capacity of the index.
    pub(crate) fn finish_index(&mut self) {
        self.directories = HashMap::new();
        self.members.shrink_to_fit();
        self.names.shrink_to_fit();
        self.zip_members.shrink_to_fit();
    }

    /// Add the bounded declared length to the just-indexed regular file.
    pub(crate) fn indexed_size(&mut self, size: u64) -> Result<u64, Error> {
        self.total = self.total.checked_add(size).ok_or_else(Error::limit)?;
        if size > self.limits.max_entry || self.total > self.limits.max_total {
            return Err(Error::limit());
        }
        let id = self.members.len() as u64 + 1;
        let member = self.members.last_mut().ok_or_else(Error::limit)?;
        member.size = size;
        member.sized = true;
        Ok(id)
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
        let size = data.as_ref().map(|v| v.length);
        self.total = self
            .total
            .checked_add(size.unwrap_or(0))
            .ok_or_else(Error::limit)?;
        if size.unwrap_or(0) > self.limits.max_entry || self.total > self.limits.max_total {
            return Err(Error::limit());
        }
        // `add` bounds both names, which a lossy label can at most triple.
        let (Ok(name_length), Ok(label_length)) =
            (u32::try_from(name.len()), u32::try_from(display.len()))
        else {
            return Err(Error::limit());
        };
        // The index is the worker's largest allocation: growing it past the worker's
        // memory fails this archive with a reason instead of ending the worker.
        if self.members.try_reserve(1).is_err()
            || self.names.try_reserve(name.len() + display.len()).is_err()
        {
            return Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "This archive has more members than the archive worker's memory \
                     holds ({} indexed)",
                    self.members.len()
                ),
            ));
        }
        let position = self.members.len();
        let id = position as u64 + 2;
        let start = self.names.len();
        self.names.extend_from_slice(name);
        let display_length = if display.as_bytes() == name {
            SAME_DISPLAY
        } else {
            self.names.extend_from_slice(display.as_bytes());
            label_length
        };
        self.children.entry(parent).or_default().push(position);
        self.members.push(Member {
            parent,
            name: start,
            name_length,
            display_length,
            size: size.unwrap_or(0),
            modified: None,
            kind,
            readable,
            sized: size.is_some(),
        });
        if let Some(data) = data {
            self.contents.insert(id, data);
        }
        Ok(id)
    }
}
