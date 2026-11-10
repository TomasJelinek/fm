use crate::{Address, AddressEncoding, WireError};

mod windows;

/// Result of explicit user activation, never of restoration, hover or metadata lookup.
/// The host keeps the current provider context until navigation succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationTarget {
    /// A location owned by the same provider; it is not a host filesystem path.
    WithinProvider(Address),
    /// A real absolute path supplied by the OS. The host checks its platform,
    /// keeps the source navigation context and delegates to its native provider.
    Native(Address),
}

impl ActivationTarget {
    /// Validate the navigation shape independently of the wire codec. This does
    /// not establish that the path exists; the native provider must still open it.
    pub fn validate(&self) -> Result<(), WireError> {
        let Self::Native(address) = self else {
            return Ok(());
        };
        let absolute = match address.encoding {
            AddressEncoding::Opaque => false,
            AddressEncoding::UnixBytes => {
                address.bytes.first() == Some(&b'/') && !address.bytes.contains(&0)
            }
            AddressEncoding::WindowsUtf16Le => {
                let (pairs, remainder) = address.bytes.as_chunks::<2>();
                if !remainder.is_empty() {
                    return Err(WireError::Malformed);
                }
                let units: Vec<u16> = pairs.iter().copied().map(u16::from_le_bytes).collect();
                windows::absolute(&units)
            }
        };
        if absolute {
            Ok(())
        } else {
            Err(WireError::Malformed)
        }
    }
}
