//! Shared bounded message codec. Every typed payload validates before use.
use crate::wire::{self, WireError};
use minicbor::{Decode, Encode};

pub trait Model: Encode<()> + for<'bytes> Decode<'bytes, ()> {
    fn validate(&self) -> Result<(), WireError>;
}

pub fn encode<T: Model>(model: &T) -> Result<Vec<u8>, WireError> {
    model.validate()?;
    let mut encoder = minicbor::Encoder::new(BoundedBytes(Vec::new()));
    model.encode(&mut encoder, &mut ()).map_err(|error| {
        if error.is_write() {
            WireError::Limit
        } else {
            WireError::Malformed
        }
    })?;
    let bytes = encoder.into_writer().0;
    wire::validate(&bytes)?;
    Ok(bytes)
}

pub fn decode<T: Model>(bytes: &[u8]) -> Result<T, WireError> {
    wire::validate(bytes)?;
    let model: T = minicbor::decode(bytes)?;
    model.validate()?;
    Ok(model)
}

struct BoundedBytes(Vec<u8>);

impl minicbor::encode::Write for BoundedBytes {
    type Error = WireError;

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let length = self
            .0
            .len()
            .checked_add(bytes.len())
            .ok_or(WireError::Limit)?;
        if length > wire::MAX_MESSAGE_BYTES {
            return Err(WireError::Limit);
        }
        if length > self.0.capacity() {
            let capacity = length.next_power_of_two().min(wire::MAX_MESSAGE_BYTES);
            self.0
                .try_reserve_exact(capacity - self.0.len())
                .map_err(|_| WireError::Limit)?;
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}
