#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AddressEncoding {
    Opaque = 0,
    WindowsUtf16Le = 1,
    UnixBytes = 2,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub encoding: AddressEncoding,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NameEncoding {
    Opaque = 0,
    Utf8 = 1,
    WindowsUtf16Le = 2,
    UnixBytes = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Directory = 1,
    File = 2,
    DirectoryLink = 3,
    Symlink = 4,
    Other = 5,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    #[default]
    Unknown = 0,
    Uncompressed = 1,
    Compressed = 2,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub size: Option<u64>,
    pub modified: Option<Timestamp>,
    pub native_attributes: Option<u32>,
    pub unix_mode: Option<u32>,
    pub compression: Compression,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: u64,
    pub address: Address,
    pub name_encoding: NameEncoding,
    pub raw_name: Vec<u8>,
    pub display: String,
    pub kind: Kind,
    pub metadata: Metadata,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    pub id: String,
    pub display: String,
    pub root: Address,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum StorageKind {
    #[default]
    Unknown = 0,
    LocalSsd = 1,
    Rotational = 2,
    Network = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum ErrorCode {
    Invalid = 1,
    Unsupported = 2,
    NotFound = 3,
    PermissionDenied = 4,
    SourceChanged = 5,
    Cancelled = 6,
    ResourceLimit = 7,
    Io = 8,
    StaleReference = 9,
    Busy = 10,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: ErrorCode,
    pub os_code: Option<i32>,
    pub message: String,
}
impl Failure {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            os_code: None,
            message: message.into(),
        }
    }
    pub fn from_io(error: std::io::Error) -> Self {
        let code = match error.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            std::io::ErrorKind::Unsupported => ErrorCode::Unsupported,
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => {
                ErrorCode::Invalid
            }
            _ => ErrorCode::Io,
        };
        Self {
            code,
            os_code: error.raw_os_error(),
            message: error.to_string(),
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for Failure {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Operation {
    Resolve = 1,
    Stat = 2,
    ListBegin = 3,
    ListNext = 4,
    CloseList = 5,
    Parent = 6,
    OpenRead = 7,
    ValidateRead = 8,
    ReadAt = 9,
    CloseRead = 10,
    CloseMount = 11,
    Volumes = 12,
    StorageKind = 13,
    SpaceInfo = 14,
    VolumeLabel = 15,
    VolumeInformation = 16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Resolve {
        address: Address,
    },
    Stat {
        mount: u64,
        node: u64,
    },
    ListBegin {
        mount: u64,
        directory: u64,
        max_entries: u32,
    },
    ListNext {
        cursor: u64,
        expected_page: u64,
        max_entries: u32,
    },
    CloseList {
        cursor: u64,
    },
    Parent {
        mount: u64,
        node: u64,
    },
    OpenRead {
        mount: u64,
        node: u64,
    },
    ValidateRead {
        reader: u64,
    },
    ReadAt {
        reader: u64,
        offset: u64,
        length: u32,
    },
    CloseRead {
        reader: u64,
    },
    CloseMount {
        mount: u64,
    },
    Volumes,
    StorageKind {
        address: Address,
    },
    SpaceInfo {
        address: Address,
    },
    VolumeLabel {
        address: Address,
    },
    VolumeInformation {
        address: Address,
    },
}
impl Request {
    pub fn operation(&self) -> Operation {
        match self {
            Self::Resolve { .. } => Operation::Resolve,
            Self::Stat { .. } => Operation::Stat,
            Self::ListBegin { .. } => Operation::ListBegin,
            Self::ListNext { .. } => Operation::ListNext,
            Self::CloseList { .. } => Operation::CloseList,
            Self::Parent { .. } => Operation::Parent,
            Self::OpenRead { .. } => Operation::OpenRead,
            Self::ValidateRead { .. } => Operation::ValidateRead,
            Self::ReadAt { .. } => Operation::ReadAt,
            Self::CloseRead { .. } => Operation::CloseRead,
            Self::CloseMount { .. } => Operation::CloseMount,
            Self::Volumes => Operation::Volumes,
            Self::StorageKind { .. } => Operation::StorageKind,
            Self::SpaceInfo { .. } => Operation::SpaceInfo,
            Self::VolumeLabel { .. } => Operation::VolumeLabel,
            Self::VolumeInformation { .. } => Operation::VolumeInformation,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Unit,
    Resolved {
        mount: u64,
        node: Node,
    },
    Node(Node),
    Page {
        cursor: u64,
        page: u64,
        done: bool,
        entries: Vec<Node>,
    },
    Parent(Option<Node>),
    Opened {
        reader: u64,
        length: u64,
        revision: Vec<u8>,
    },
    Content {
        bytes: Vec<u8>,
        eof: bool,
    },
    Volumes(Vec<Volume>),
    StorageKind(StorageKind),
    SpaceInfo {
        available: u64,
        total: u64,
    },
    Failed(Failure),
    VolumeLabel(Option<crate::NativeText>),
    VolumeInformation(crate::VolumeInformation),
}
