//! Optional read-only volume details. Native text never serves as a display path.
use crate::{Address, NameEncoding, StorageKind};

pub const MAX_VOLUME_TEXT: usize = 64 * 1024;
pub const MAX_VOLUME_DIAGNOSTICS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeText {
    pub encoding: NameEncoding,
    pub bytes: Vec<u8>,
}

/// Independent measured values; `total` is physical filesystem capacity, not a quota.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeCapacity {
    pub total: Option<u64>,
    pub free: Option<u64>,
    pub available: Option<u64>,
}

/// Only properties positively reported by the operating system are included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VolumeProperty {
    CasePreservedNames = 1,
    CaseSensitiveLookup = 2,
    UnicodeNames = 3,
    PersistentAcls = 4,
    FileCompression = 5,
    SparseFiles = 6,
    NamedStreams = 7,
    HardLinks = 8,
    ReparsePoints = 9,
    Encryption = 10,
    Quotas = 11,
    NoExecute = 12,
    NoSetId = 13,
    NoDeviceFiles = 14,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VolumeInformation {
    pub root: Option<Address>,
    pub label: Option<NativeText>,
    pub filesystem: Option<NativeText>,
    pub source: Option<NativeText>,
    pub capacity: VolumeCapacity,
    pub storage: StorageKind,
    pub read_only: Option<bool>,
    pub properties: Vec<VolumeProperty>,
    /// Partial failures preserve independently measured fields.
    pub unavailable: Vec<String>,
}
