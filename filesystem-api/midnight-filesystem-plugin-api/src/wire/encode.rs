use super::{PREFIX, validate};
use crate::*;

pub fn encode_request(value: &Request, maximum: usize) -> Result<Vec<u8>, WireError> {
    validate::request(value)?;
    let mut out = Writer::new(maximum)?;
    match value {
        Request::Resolve { address }
        | Request::StorageKind { address }
        | Request::SpaceInfo { address }
        | Request::VolumeLabel { address }
        | Request::VolumeInformation { address } => out.address(address)?,
        Request::Stat { mount, node }
        | Request::Activate { mount, node }
        | Request::Refresh { mount, node }
        | Request::Parent { mount, node }
        | Request::OpenRead { mount, node } => {
            out.u64(*mount)?;
            out.u64(*node)?;
        }
        Request::ListBegin {
            mount,
            directory,
            max_entries,
        } => {
            out.u64(*mount)?;
            out.u64(*directory)?;
            out.u32(*max_entries)?;
        }
        Request::ListNext {
            cursor,
            expected_page,
            max_entries,
        } => {
            out.u64(*cursor)?;
            out.u64(*expected_page)?;
            out.u32(*max_entries)?;
        }
        Request::CloseList { cursor } => out.u64(*cursor)?,
        Request::CloseRead { reader } | Request::ValidateRead { reader } => out.u64(*reader)?,
        Request::CloseMount { mount } => out.u64(*mount)?,
        Request::ReadAt {
            reader,
            offset,
            length,
        } => {
            out.u64(*reader)?;
            out.u64(*offset)?;
            out.u32(*length)?;
        }
        Request::Volumes => {}
        Request::ResolveText { text } => out.text(text)?,
    }
    Ok(out.bytes)
}

pub fn encode_response(value: &Response, maximum: usize) -> Result<Vec<u8>, WireError> {
    validate::response(value)?;
    let mut out = Writer::new(maximum)?;
    match value {
        Response::Activated(target) => {
            out.u16(14)?;
            let (kind, address) = match target {
                ActivationTarget::WithinProvider(address) => (1, address),
                ActivationTarget::Native(address) => (2, address),
            };
            out.u8(kind)?;
            out.address(address)?;
        }
        Response::Unit => out.u16(1)?,
        Response::Resolved { mount, node } => {
            out.u16(2)?;
            out.u64(*mount)?;
            out.node(node)?;
        }
        Response::Node(node) => {
            out.u16(3)?;
            out.node(node)?;
        }
        Response::Page {
            cursor,
            page,
            done,
            entries,
        } => {
            out.u16(4)?;
            out.u64(*cursor)?;
            out.u64(*page)?;
            out.bool(*done)?;
            out.u32(entries.len() as u32)?;
            for node in entries {
                out.node(node)?;
            }
        }
        Response::Parent(node) => {
            out.u16(5)?;
            out.bool(node.is_some())?;
            if let Some(node) = node {
                out.node(node)?;
            }
        }
        Response::Opened {
            reader,
            length,
            revision,
        } => {
            out.u16(6)?;
            out.u64(*reader)?;
            out.u64(*length)?;
            out.blob(revision)?;
        }
        Response::Content { bytes, eof } => {
            out.u16(7)?;
            out.blob(bytes)?;
            out.bool(*eof)?;
        }
        Response::Volumes(volumes) => {
            out.u16(8)?;
            out.u32(volumes.len() as u32)?;
            for volume in volumes {
                out.text(&volume.id)?;
                out.text(&volume.display)?;
                out.address(&volume.root)?;
            }
        }
        Response::StorageKind(kind) => {
            out.u16(9)?;
            out.u8(*kind as u8)?;
        }
        Response::SpaceInfo { available, total } => {
            out.u16(10)?;
            out.u64(*available)?;
            out.u64(*total)?;
        }
        Response::Failed(failure) => {
            out.u16(11)?;
            out.u16(failure.code as u16)?;
            out.bool(failure.os_code.is_some())?;
            if let Some(code) = failure.os_code {
                out.put(&code.to_le_bytes())?;
            }
            out.text(&failure.message)?;
        }
        Response::VolumeLabel(label) => {
            out.u16(12)?;
            out.native_text(label)?;
        }
        Response::VolumeInformation(info) => {
            out.u16(13)?;
            out.bool(info.root.is_some())?;
            if let Some(root) = &info.root {
                out.address(root)?;
            }
            out.native_text(&info.label)?;
            out.native_text(&info.filesystem)?;
            out.native_text(&info.source)?;
            for value in [
                info.capacity.total,
                info.capacity.free,
                info.capacity.available,
            ] {
                out.bool(value.is_some())?;
                if let Some(value) = value {
                    out.u64(value)?;
                }
            }
            out.u8(info.storage as u8)?;
            out.bool(info.read_only.is_some())?;
            if let Some(value) = info.read_only {
                out.bool(value)?;
            }
            out.u32(info.properties.len() as u32)?;
            for property in &info.properties {
                out.u8(*property as u8)?;
            }
            out.u32(info.unavailable.len() as u32)?;
            for diagnostic in &info.unavailable {
                out.text(diagnostic)?;
            }
        }
    }
    Ok(out.bytes)
}

struct Writer {
    bytes: Vec<u8>,
    maximum: usize,
}
impl Writer {
    fn new(maximum: usize) -> Result<Self, WireError> {
        let mut out = Self {
            bytes: vec![],
            maximum,
        };
        out.put(PREFIX)?;
        Ok(out)
    }
    fn put(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        let size = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(WireError::Limit)?;
        if size > self.maximum {
            return Err(WireError::Limit);
        }
        self.bytes
            .try_reserve_exact(bytes.len())
            .map_err(|_| WireError::Limit)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn u8(&mut self, value: u8) -> Result<(), WireError> {
        self.put(&[value])
    }
    fn u16(&mut self, value: u16) -> Result<(), WireError> {
        self.put(&value.to_le_bytes())
    }
    fn u32(&mut self, value: u32) -> Result<(), WireError> {
        self.put(&value.to_le_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<(), WireError> {
        self.put(&value.to_le_bytes())
    }
    fn bool(&mut self, value: bool) -> Result<(), WireError> {
        self.u8(u8::from(value))
    }
    fn blob(&mut self, value: &[u8]) -> Result<(), WireError> {
        self.u64(value.len() as u64)?;
        self.put(value)
    }
    fn text(&mut self, value: &str) -> Result<(), WireError> {
        self.blob(value.as_bytes())
    }
    fn address(&mut self, value: &Address) -> Result<(), WireError> {
        self.u8(value.encoding as u8)?;
        self.blob(&value.bytes)
    }
    fn native_text(&mut self, value: &Option<NativeText>) -> Result<(), WireError> {
        self.bool(value.is_some())?;
        if let Some(value) = value {
            self.u8(value.encoding as u8)?;
            self.blob(&value.bytes)?;
        }
        Ok(())
    }
    fn metadata(&mut self, value: &Metadata) -> Result<(), WireError> {
        self.u16(
            u16::from(value.size.is_some())
                | (u16::from(value.modified.is_some()) << 1)
                | (u16::from(value.native_attributes.is_some()) << 2)
                | (u16::from(value.unix_mode.is_some()) << 3),
        )?;
        if let Some(size) = value.size {
            self.u64(size)?;
        }
        if let Some(time) = value.modified {
            self.put(&time.seconds.to_le_bytes())?;
            self.u32(time.nanos)?;
        }
        if let Some(attributes) = value.native_attributes {
            self.u32(attributes)?;
        }
        if let Some(mode) = value.unix_mode {
            self.u32(mode)?;
        }
        self.u8(value.compression as u8)
    }
    fn node(&mut self, value: &Node) -> Result<(), WireError> {
        self.u64(value.id)?;
        self.address(&value.address)?;
        self.u8(value.name_encoding as u8)?;
        self.blob(&value.raw_name)?;
        self.text(&value.display)?;
        self.u8(value.kind as u8)?;
        self.metadata(&value.metadata)
    }
}
