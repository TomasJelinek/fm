use super::*;
use windows_sys::Win32::NetworkManagement::WNet::*;
// winnetwk.h values; windows-sys exposes these three under its unrelated WinSock feature.
pub(super) const DISPLAY_DOMAIN: u32 = 1;
pub(super) const DISPLAY_SERVER: u32 = 2;
pub(super) const DISPLAY_SHARE: u32 = 3;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Resource {
    pub scope: u32,
    pub kind: u32,
    pub display_type: u32,
    pub usage: u32,
    pub local: Option<Vec<u16>>,
    pub remote: Option<Vec<u16>>,
    pub comment: Option<Vec<u16>>,
    pub provider: Option<Vec<u16>>,
}
pub(super) const MAX_KEY_BYTES: usize = 64 * 1024;
impl Resource {
    pub fn retained_bytes(&self) -> usize {
        [&self.local, &self.remote, &self.comment, &self.provider]
            .iter()
            .fold(std::mem::size_of::<Self>(), |bytes, field| {
                bytes.saturating_add(
                    field
                        .as_ref()
                        .map_or(0, |value| value.capacity().saturating_mul(2)),
                )
            })
    }
    pub fn encode(&self) -> Result<Vec<u8>, Failure> {
        let fields = [&self.local, &self.remote, &self.comment, &self.provider];
        let bytes = fields.iter().fold(36usize, |bytes, field| {
            bytes.saturating_add(
                field
                    .as_ref()
                    .map_or(0, |value| value.len().saturating_mul(2)),
            )
        });
        if bytes > MAX_KEY_BYTES {
            return Err(limit());
        }
        if fields
            .iter()
            .any(|field| field.as_ref().is_some_and(|value| value.contains(&0)))
        {
            return Err(invalid());
        }
        let mut key = Vec::new();
        key.try_reserve_exact(bytes).map_err(|_| limit())?;
        key.extend_from_slice(b"WNR\x01");
        for value in [self.scope, self.kind, self.display_type, self.usage] {
            key.extend(value.to_le_bytes());
        }
        for field in fields {
            key.extend(
                field
                    .as_ref()
                    .map_or(u32::MAX, |value| value.len() as u32)
                    .to_le_bytes(),
            );
            if let Some(value) = field {
                key.extend(value.iter().flat_map(|unit| unit.to_le_bytes()));
            }
        }
        Ok(key)
    }
    pub fn decode(key: &[u8]) -> Result<Self, Failure> {
        if key.len() > MAX_KEY_BYTES {
            return Err(limit());
        }
        let mut input = Reader(key.strip_prefix(b"WNR\x01").ok_or_else(invalid)?);
        let value = Self {
            scope: input.number()?,
            kind: input.number()?,
            display_type: input.number()?,
            usage: input.number()?,
            local: input.text()?,
            remote: input.text()?,
            comment: input.text()?,
            provider: input.text()?,
        };
        if !input.0.is_empty() {
            return Err(invalid());
        }
        Ok(value)
    }
    pub fn classify(&self) -> Result<Option<NodeKind>, Failure> {
        if !(RESOURCE_CONNECTED..=RESOURCE_CONTEXT).contains(&self.scope) {
            return Err(invalid());
        }
        if self.kind == RESOURCETYPE_PRINT || self.display_type == RESOURCEDISPLAYTYPE_SHAREADMIN {
            return Ok(None);
        }
        let remote = self.remote.as_ref().is_some_and(|name| !name.is_empty());
        let provider = self.provider.as_ref().is_some_and(|name| !name.is_empty());
        if self.display_type == DISPLAY_SERVER && remote {
            return Ok(Some(NodeKind::Server));
        }
        if remote
            && (self.display_type == DISPLAY_SHARE
                || (self.kind == RESOURCETYPE_DISK && self.usage & RESOURCEUSAGE_CONNECTABLE != 0))
        {
            return Ok(Some(NodeKind::Share));
        }
        if (remote || provider)
            && (self.usage & RESOURCEUSAGE_CONTAINER != 0
                || matches!(
                    self.display_type,
                    DISPLAY_DOMAIN
                        | RESOURCEDISPLAYTYPE_NETWORK
                        | RESOURCEDISPLAYTYPE_ROOT
                        | RESOURCEDISPLAYTYPE_NDSCONTAINER
                ))
        {
            return Ok(Some(NodeKind::Container));
        }
        Ok(None)
    }
    pub fn record(&self) -> Result<Option<Record>, Failure> {
        let Some(kind) = self.classify()? else {
            return Ok(None);
        };
        let name = self
            .remote
            .as_deref()
            .filter(|name| !name.is_empty())
            .or(self.provider.as_deref())
            .ok_or_else(invalid)?;
        let label = name
            .rsplit(|unit| *unit == 92)
            .find(|part| !part.is_empty())
            .unwrap_or(name);
        Ok(Some(Record {
            target: Target {
                kind,
                key: self.encode()?,
            },
            display: String::from_utf16_lossy(label),
        }))
    }
}

struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn number(&mut self) -> Result<u32, Failure> {
        let (head, tail) = self.0.split_at_checked(4).ok_or_else(invalid)?;
        self.0 = tail;
        Ok(u32::from_le_bytes(head.try_into().map_err(|_| invalid())?))
    }
    fn text(&mut self) -> Result<Option<Vec<u16>>, Failure> {
        let count = self.number()?;
        if count == u32::MAX {
            return Ok(None);
        }
        let bytes = (count as usize).checked_mul(2).ok_or_else(invalid)?;
        let (head, tail) = self.0.split_at_checked(bytes).ok_or_else(invalid)?;
        self.0 = tail;
        let mut units = Vec::new();
        units
            .try_reserve_exact(count as usize)
            .map_err(|_| limit())?;
        units.extend(
            head.as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
        );
        if units.contains(&0) {
            return Err(invalid());
        }
        Ok(Some(units))
    }
}
