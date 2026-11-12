//! Prepared same-parent Rename. Opaque identities describe native pins acquired
//! before confirmation; Browse node IDs do not attest listing-era object identity.
use crate::{
    Address, AddressEncoding, Failure, Kind, Metadata, NameEncoding, NativeText, WireError,
};

pub const MAX_RENAME_NAME_BYTES: usize = 64 * 1024;
pub const MAX_RENAME_IDENTITY_BYTES: usize = 256;
pub const MAX_RENAME_RECOVERY_RECORDS: usize = 4;
pub const MAX_RENAME_RECOVERY_ADDRESS_BYTES: usize = 256 * 1024;
/// Upper bound for any valid receipt, including the longest raw name/locators.
pub const MAX_RENAME_RECEIPT_BYTES: usize = 384 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameObject {
    pub identity: Vec<u8>,
    pub revision: Vec<u8>,
    pub name: NativeText,
    pub kind: Kind,
    pub metadata: Metadata,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedRename {
    pub handle: u64,
    pub source: RenameObject,
    pub parent_identity: Vec<u8>,
    pub recoverable_replace: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenamePolicy {
    NoReplace,
    RecoverableCurrentEntry { conflict: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameConflict {
    pub token: u64,
    pub target: RenameObject,
    pub can_replace: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RenameRecoveryRole {
    Source = 0,
    DisplacedTarget = 1,
    EmptyStage = 2,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameRecovery {
    pub role: RenameRecoveryRole,
    /// Exact residual locator; this observation grants no later write authority.
    pub address: Address,
    /// Empty only if the native implementation could not confirm identity.
    pub identity: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RenameSourceLocation {
    OriginalName = 0,
    Destination = 1,
    Recovery = 2,
    Unknown = 3,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameOutcome {
    Ready,
    Executing,
    Conflict(RenameConflict),
    /// Terminal: no residual namespace change. Temporary staging may have occurred.
    Unchanged(Failure),
    Renamed {
        name: NativeText,
        same_entry: bool,
        recovery: Vec<RenameRecovery>,
        durability_confirmed: bool,
    },
    RecoveryRequired {
        source_location: RenameSourceLocation,
        destination_name: Option<NativeText>,
        recovery: Vec<RenameRecovery>,
        error: Failure,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameState {
    pub prepared: u64,
    pub attempt: u64,
    pub outcome: RenameOutcome,
}
fn require(value: bool) -> Result<(), WireError> {
    if value {
        Ok(())
    } else {
        Err(WireError::Malformed)
    }
}
fn bounded(length: usize, maximum: usize) -> Result<(), WireError> {
    if length <= maximum {
        Ok(())
    } else {
        Err(WireError::Limit)
    }
}
fn identity(value: &[u8]) -> Result<(), WireError> {
    require(!value.is_empty())?;
    bounded(value.len(), MAX_RENAME_IDENTITY_BYTES)
}
impl NativeText {
    /// Validate a lossless basename without normalization or OS access. Native
    /// adapters additionally enforce their own namespace/commit policy.
    pub fn validate_rename_observed_basename(&self) -> Result<(), WireError> {
        bounded(self.bytes.len(), MAX_RENAME_NAME_BYTES)?;
        require(!self.bytes.is_empty())?;
        match self.encoding {
            NameEncoding::Opaque => Err(WireError::Unsupported),
            NameEncoding::Utf8 | NameEncoding::UnixBytes => {
                if self.encoding == NameEncoding::Utf8 {
                    require(std::str::from_utf8(&self.bytes).is_ok())?;
                }
                require(
                    !self.bytes.contains(&0)
                        && !self.bytes.contains(&b'/')
                        && self.bytes != b"."
                        && self.bytes != b"..",
                )
            }
            NameEncoding::WindowsUtf16Le => {
                let (pairs, remainder) = self.bytes.as_chunks::<2>();
                require(remainder.is_empty())?;
                let units = || pairs.iter().copied().map(u16::from_le_bytes);
                require(!units().any(|unit| [0, 47, 92].contains(&unit)))?;
                require(!units().eq([46]) && !units().eq([46, 46]))
            }
        }
    }
    /// A newly proposed destination additionally follows the platform's normal
    /// Rename policy. Existing extended Windows spellings remain observable.
    pub fn validate_rename_basename(&self) -> Result<(), WireError> {
        self.validate_rename_observed_basename()?;
        bounded(self.bytes.len(), MAX_RENAME_NAME_BYTES)?;
        require(!self.bytes.is_empty())?;
        match self.encoding {
            NameEncoding::Opaque => Err(WireError::Unsupported),
            NameEncoding::Utf8 | NameEncoding::UnixBytes => {
                if self.encoding == NameEncoding::Utf8 {
                    require(std::str::from_utf8(&self.bytes).is_ok())?;
                }
                require(
                    !self.bytes.contains(&0)
                        && !self.bytes.contains(&b'/')
                        && self.bytes != b"."
                        && self.bytes != b"..",
                )
            }
            NameEncoding::WindowsUtf16Le => {
                let (pairs, remainder) = self.bytes.as_chunks::<2>();
                require(remainder.is_empty())?;
                let units = || pairs.iter().copied().map(u16::from_le_bytes);
                require(!units().any(|unit| {
                    unit < 32 || b"<>:\"/\\|?*".iter().any(|byte| unit == u16::from(*byte))
                }))?;
                require(
                    !units()
                        .next_back()
                        .is_some_and(|unit| unit == 46 || unit == 32),
                )?;
                let stem = || {
                    units().take_while(|unit| *unit != 46).map(|unit| {
                        if (97..=122).contains(&unit) {
                            unit - 32
                        } else {
                            unit
                        }
                    })
                };
                require(
                    !["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
                        .iter()
                        .any(|value| stem().eq(value.encode_utf16())),
                )?;
                if stem().count() == 4 {
                    let port = stem().take(3).eq("COM".encode_utf16())
                        || stem().take(3).eq("LPT".encode_utf16());
                    let digit = stem().nth(3).is_some_and(|unit| {
                        (49..=57).contains(&unit) || [0xb9, 0xb2, 0xb3].contains(&unit)
                    });
                    require(!port || !digit)?;
                }
                Ok(())
            }
        }
    }
}
impl RenameObject {
    pub fn validate(&self) -> Result<(), WireError> {
        identity(&self.identity)?;
        require(!self.revision.is_empty())?;
        bounded(self.revision.len(), crate::MAX_REVISION)?;
        self.name.validate_rename_observed_basename()?;
        require(self.kind != Kind::Other)?;
        require(
            self.metadata
                .modified
                .is_none_or(|time| time.nanos < 1_000_000_000),
        )?;
        require(self.kind == Kind::File || self.metadata.size.is_none())
    }
}
impl PreparedRename {
    pub fn validate(&self) -> Result<(), WireError> {
        require(self.handle != 0)?;
        self.source.validate()?;
        identity(&self.parent_identity)?;
        require(!self.recoverable_replace || self.source.kind == Kind::File)
    }
}
fn recovery(values: &[RenameRecovery]) -> Result<(), WireError> {
    bounded(values.len(), MAX_RENAME_RECOVERY_RECORDS)?;
    let mut total = 0usize;
    for value in values {
        total = total
            .checked_add(value.address.bytes.len())
            .ok_or(WireError::Limit)?;
        bounded(total, MAX_RENAME_RECOVERY_ADDRESS_BYTES)?;
        bounded(value.identity.len(), MAX_RENAME_IDENTITY_BYTES)?;
        require(!value.address.bytes.is_empty())?;
        match value.address.encoding {
            AddressEncoding::Opaque => {}
            AddressEncoding::UnixBytes => require(!value.address.bytes.contains(&0))?,
            AddressEncoding::WindowsUtf16Le => {
                let (units, remainder) = value.address.bytes.as_chunks::<2>();
                require(remainder.is_empty() && !units.contains(&[0, 0]))?;
            }
        }
    }
    Ok(())
}
impl RenameOutcome {
    /// Structural validation only; the retained native pin owns identity and
    /// namespace checks. This performs no allocation and no filesystem I/O.
    pub fn validate(&self) -> Result<(), WireError> {
        match self {
            Self::Ready | Self::Executing => Ok(()),
            Self::Conflict(value) => {
                require(value.token != 0)?;
                value.target.validate()?;
                require(!value.can_replace || value.target.kind == Kind::File)
            }
            Self::Unchanged(error) => bounded(error.message.len(), crate::MAX_DIAGNOSTIC),
            Self::Renamed {
                name,
                recovery: values,
                ..
            } => {
                name.validate_rename_basename()?;
                recovery(values)
            }
            Self::RecoveryRequired {
                destination_name,
                recovery: values,
                error,
                ..
            } => {
                if let Some(name) = destination_name {
                    name.validate_rename_basename()?;
                }
                recovery(values)?;
                bounded(error.message.len(), crate::MAX_DIAGNOSTIC)
            }
        }
    }
}
impl RenameState {
    pub fn validate(&self) -> Result<(), WireError> {
        require(self.prepared != 0)?;
        require((self.attempt == 0) == matches!(self.outcome, RenameOutcome::Ready))?;
        self.outcome.validate()
    }
}
