use crate::validation::{check, invalid};
use crate::*;
use std::io;

pub(crate) struct Encoder(pub Vec<u8>);

impl Encoder {
    pub fn new(header: Header) -> Self {
        let mut value = Self(Vec::with_capacity(128));
        value.u64(header.session);
        value.u64(header.document);
        value.u64(header.sequence);
        value
    }
    pub fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }
    pub fn u32(&mut self, value: u32) {
        self.0.extend(value.to_le_bytes());
    }
    pub fn u64(&mut self, value: u64) {
        self.0.extend(value.to_le_bytes());
    }
    pub fn f64(&mut self, value: f64) {
        self.0.extend(value.to_le_bytes());
    }
    pub fn text(&mut self, value: &str) -> io::Result<()> {
        check(value.len() <= MAX_CONTROL_BYTES.saturating_sub(self.0.len() + 4))?;
        self.u32(value.len() as u32);
        self.0.extend(value.as_bytes());
        Ok(())
    }
    pub fn viewport(&mut self, value: Viewport) {
        self.u64(value.revision);
        self.u32(value.width);
        self.u32(value.height);
        self.f64(value.scale);
    }
    pub fn policy(&mut self, value: Policy) {
        self.bool(value.network);
        self.bool(value.scripts);
    }
    pub fn resource(&mut self, value: &Resource) -> io::Result<()> {
        self.u64(value.id);
        self.u64(value.length);
        self.text(&value.mime)
    }
    pub fn modifiers(&mut self, value: Modifiers) {
        self.u8(u8::from(value.shift)
            | u8::from(value.control) << 1
            | u8::from(value.alt) << 2
            | u8::from(value.meta) << 3);
    }
}

pub(crate) struct Decoder<'a>(pub &'a [u8]);

impl Decoder<'_> {
    pub fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let bytes = self.0.get(..N).ok_or_else(invalid)?;
        let value = bytes.try_into().map_err(|_| invalid())?;
        self.0 = &self.0[N..];
        Ok(value)
    }
    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub fn bool(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid()),
        }
    }
    pub fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub fn f64(&mut self) -> io::Result<f64> {
        Ok(f64::from_le_bytes(self.array()?))
    }
    pub fn text(&mut self) -> io::Result<String> {
        let length = self.u32()? as usize;
        let bytes = self.0.get(..length).ok_or_else(invalid)?;
        let value = std::str::from_utf8(bytes)
            .map_err(|_| invalid())?
            .to_owned();
        self.0 = &self.0[length..];
        Ok(value)
    }
    pub fn header(&mut self) -> io::Result<Header> {
        Header {
            session: self.u64()?,
            document: self.u64()?,
            sequence: self.u64()?,
        }
        .validate()
    }
    pub fn viewport(&mut self) -> io::Result<Viewport> {
        Viewport {
            revision: self.u64()?,
            width: self.u32()?,
            height: self.u32()?,
            scale: self.f64()?,
        }
        .validate()
    }
    pub fn policy(&mut self) -> io::Result<Policy> {
        Policy {
            network: self.bool()?,
            scripts: self.bool()?,
        }
        .validate()
    }
    pub fn resource(&mut self) -> io::Result<Resource> {
        let value = Resource {
            id: self.u64()?,
            length: self.u64()?,
            mime: self.text()?,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn modifiers(&mut self) -> io::Result<Modifiers> {
        let bits = self.u8()?;
        check(bits & !15 == 0)?;
        Ok(Modifiers {
            shift: bits & 1 != 0,
            control: bits & 2 != 0,
            alt: bits & 4 != 0,
            meta: bits & 8 != 0,
        })
    }
}
