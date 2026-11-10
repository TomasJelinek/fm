use super::{PREFIX, validate};
use crate::*;

pub fn decode_request(operation: u32, bytes: &[u8], maximum: usize) -> Result<Request, WireError> {
    let mut input = Reader::new(bytes, maximum)?;
    let value = match operation {
        1 => Request::Resolve {
            address: input.address()?,
        },
        2 => Request::Stat {
            mount: input.u64()?,
            node: input.u64()?,
        },
        3 => Request::ListBegin {
            mount: input.u64()?,
            directory: input.u64()?,
            max_entries: input.u32()?,
        },
        4 => Request::ListNext {
            cursor: input.u64()?,
            expected_page: input.u64()?,
            max_entries: input.u32()?,
        },
        5 => Request::CloseList {
            cursor: input.u64()?,
        },
        6 => Request::Parent {
            mount: input.u64()?,
            node: input.u64()?,
        },
        7 => Request::OpenRead {
            mount: input.u64()?,
            node: input.u64()?,
        },
        8 => Request::ValidateRead {
            reader: input.u64()?,
        },
        9 => Request::ReadAt {
            reader: input.u64()?,
            offset: input.u64()?,
            length: input.u32()?,
        },
        10 => Request::CloseRead {
            reader: input.u64()?,
        },
        11 => Request::CloseMount {
            mount: input.u64()?,
        },
        12 => Request::Volumes,
        13 => Request::StorageKind {
            address: input.address()?,
        },
        14 => Request::SpaceInfo {
            address: input.address()?,
        },
        15 => Request::VolumeLabel {
            address: input.address()?,
        },
        16 => Request::VolumeInformation {
            address: input.address()?,
        },
        17 => Request::Activate {
            mount: input.u64()?,
            node: input.u64()?,
        },
        18 => Request::Refresh {
            mount: input.u64()?,
            node: input.u64()?,
        },
        19 => Request::ResolveText {
            text: input.text()?,
        },
        _ => return Err(WireError::Unsupported),
    };
    input.finish()?;
    validate::request(&value)?;
    Ok(value)
}
pub fn decode_response(bytes: &[u8], maximum: usize) -> Result<Response, WireError> {
    let mut input = Reader::new(bytes, maximum)?;
    let value = match input.u16()? {
        1 => Response::Unit,
        2 => Response::Resolved {
            mount: input.u64()?,
            node: input.node()?,
        },
        3 => Response::Node(input.node()?),
        4 => {
            let cursor = input.u64()?;
            let page = input.u64()?;
            let done = input.bool()?;
            let count = input.count(MAX_BATCH as usize)?;
            let mut entries = Vec::new();
            entries
                .try_reserve_exact(count)
                .map_err(|_| WireError::Limit)?;
            for _ in 0..count {
                entries.push(input.node()?);
            }
            Response::Page {
                cursor,
                page,
                done,
                entries,
            }
        }
        5 => Response::Parent(if input.bool()? {
            Some(input.node()?)
        } else {
            None
        }),
        6 => Response::Opened {
            reader: input.u64()?,
            length: input.u64()?,
            revision: input.blob()?,
        },
        7 => Response::Content {
            bytes: input.blob()?,
            eof: input.bool()?,
        },
        8 => {
            let count = input.count(4096)?;
            let mut volumes = Vec::new();
            volumes
                .try_reserve_exact(count)
                .map_err(|_| WireError::Limit)?;
            for _ in 0..count {
                volumes.push(Volume {
                    id: input.text()?,
                    display: input.text()?,
                    root: input.address()?,
                });
            }
            Response::Volumes(volumes)
        }
        9 => Response::StorageKind(match input.u8()? {
            0 => StorageKind::Unknown,
            1 => StorageKind::LocalSsd,
            2 => StorageKind::Rotational,
            3 => StorageKind::Network,
            _ => return Err(WireError::Unsupported),
        }),
        10 => Response::SpaceInfo {
            available: input.u64()?,
            total: input.u64()?,
        },
        11 => {
            let code = match input.u16()? {
                1 => ErrorCode::Invalid,
                2 => ErrorCode::Unsupported,
                3 => ErrorCode::NotFound,
                4 => ErrorCode::PermissionDenied,
                5 => ErrorCode::SourceChanged,
                6 => ErrorCode::Cancelled,
                7 => ErrorCode::ResourceLimit,
                8 => ErrorCode::Io,
                9 => ErrorCode::StaleReference,
                10 => ErrorCode::Busy,
                11 => ErrorCode::Unavailable,
                12 => ErrorCode::Offline,
                13 => ErrorCode::AuthenticationRequired,
                14 => ErrorCode::MountRequired,
                15 => ErrorCode::TimedOut,
                _ => return Err(WireError::Unsupported),
            };
            let os_code = if input.bool()? {
                Some(i32::from_le_bytes(input.array()?))
            } else {
                None
            };
            Response::Failed(Failure {
                code,
                os_code,
                message: input.text()?,
            })
        }
        12 => Response::VolumeLabel(input.native_text()?),
        13 => Response::VolumeInformation(input.volume_information()?),
        14 => Response::Activated(match input.u8()? {
            1 => ActivationTarget::WithinProvider(input.address()?),
            2 => ActivationTarget::Native(input.address()?),
            _ => return Err(WireError::Unsupported),
        }),
        _ => return Err(WireError::Unsupported),
    };
    input.finish()?;
    validate::response(&value)?;
    Ok(value)
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], maximum: usize) -> Result<Self, WireError> {
        if bytes.len() > maximum {
            return Err(WireError::Limit);
        }
        if bytes.get(..8) != Some(PREFIX) {
            return Err(WireError::Malformed);
        }
        Ok(Self { bytes, position: 8 })
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let end = self.position.checked_add(length).ok_or(WireError::Limit)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(WireError::Malformed)?;
        self.position = end;
        Ok(bytes)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        self.take(N)?.try_into().map_err(|_| WireError::Malformed)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.array::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn bool(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WireError::Malformed),
        }
    }
    fn blob(&mut self) -> Result<Vec<u8>, WireError> {
        let length = usize::try_from(self.u64()?).map_err(|_| WireError::Limit)?;
        let bytes = self.take(length)?;
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(length)
            .map_err(|_| WireError::Limit)?;
        owned.extend_from_slice(bytes);
        Ok(owned)
    }
    fn text(&mut self) -> Result<String, WireError> {
        String::from_utf8(self.blob()?).map_err(|_| WireError::Malformed)
    }
    fn count(&mut self, maximum: usize) -> Result<usize, WireError> {
        let count = self.u32()? as usize;
        if count > maximum {
            return Err(WireError::Limit);
        }
        if count > self.bytes.len() - self.position {
            return Err(WireError::Malformed);
        }
        Ok(count)
    }
    fn address(&mut self) -> Result<Address, WireError> {
        let encoding = match self.u8()? {
            0 => AddressEncoding::Opaque,
            1 => AddressEncoding::WindowsUtf16Le,
            2 => AddressEncoding::UnixBytes,
            _ => return Err(WireError::Unsupported),
        };
        Ok(Address {
            encoding,
            bytes: self.blob()?,
        })
    }
    fn native_text(&mut self) -> Result<Option<NativeText>, WireError> {
        if !self.bool()? {
            return Ok(None);
        }
        let encoding = match self.u8()? {
            1 => NameEncoding::Utf8,
            2 => NameEncoding::WindowsUtf16Le,
            3 => NameEncoding::UnixBytes,
            _ => return Err(WireError::Unsupported),
        };
        Ok(Some(NativeText {
            encoding,
            bytes: self.blob()?,
        }))
    }
    fn optional_u64(&mut self) -> Result<Option<u64>, WireError> {
        Ok(if self.bool()? {
            Some(self.u64()?)
        } else {
            None
        })
    }
    fn volume_information(&mut self) -> Result<VolumeInformation, WireError> {
        let root = if self.bool()? {
            Some(self.address()?)
        } else {
            None
        };
        let label = self.native_text()?;
        let filesystem = self.native_text()?;
        let source = self.native_text()?;
        let capacity = VolumeCapacity {
            total: self.optional_u64()?,
            free: self.optional_u64()?,
            available: self.optional_u64()?,
        };
        let storage = match self.u8()? {
            0 => StorageKind::Unknown,
            1 => StorageKind::LocalSsd,
            2 => StorageKind::Rotational,
            3 => StorageKind::Network,
            _ => return Err(WireError::Unsupported),
        };
        let read_only = if self.bool()? {
            Some(self.bool()?)
        } else {
            None
        };
        let count = self.count(14)?;
        let mut properties = Vec::with_capacity(count);
        for _ in 0..count {
            properties.push(match self.u8()? {
                1 => VolumeProperty::CasePreservedNames,
                2 => VolumeProperty::CaseSensitiveLookup,
                3 => VolumeProperty::UnicodeNames,
                4 => VolumeProperty::PersistentAcls,
                5 => VolumeProperty::FileCompression,
                6 => VolumeProperty::SparseFiles,
                7 => VolumeProperty::NamedStreams,
                8 => VolumeProperty::HardLinks,
                9 => VolumeProperty::ReparsePoints,
                10 => VolumeProperty::Encryption,
                11 => VolumeProperty::Quotas,
                12 => VolumeProperty::NoExecute,
                13 => VolumeProperty::NoSetId,
                14 => VolumeProperty::NoDeviceFiles,
                _ => return Err(WireError::Unsupported),
            });
        }
        let count = self.count(MAX_VOLUME_DIAGNOSTICS)?;
        let mut unavailable = Vec::with_capacity(count);
        for _ in 0..count {
            unavailable.push(self.text()?);
        }
        Ok(VolumeInformation {
            root,
            label,
            filesystem,
            source,
            capacity,
            storage,
            read_only,
            properties,
            unavailable,
        })
    }
    fn metadata(&mut self) -> Result<Metadata, WireError> {
        let flags = self.u16()?;
        if flags & !15 != 0 {
            return Err(WireError::Unsupported);
        }
        Ok(Metadata {
            size: if flags & 1 != 0 {
                Some(self.u64()?)
            } else {
                None
            },
            modified: if flags & 2 != 0 {
                Some(Timestamp {
                    seconds: i64::from_le_bytes(self.array()?),
                    nanos: self.u32()?,
                })
            } else {
                None
            },
            native_attributes: if flags & 4 != 0 {
                Some(self.u32()?)
            } else {
                None
            },
            unix_mode: if flags & 8 != 0 {
                Some(self.u32()?)
            } else {
                None
            },
            compression: match self.u8()? {
                0 => Compression::Unknown,
                1 => Compression::Uncompressed,
                2 => Compression::Compressed,
                _ => return Err(WireError::Unsupported),
            },
        })
    }
    fn node(&mut self) -> Result<Node, WireError> {
        let id = self.u64()?;
        let address = self.address()?;
        let name_encoding = match self.u8()? {
            0 => NameEncoding::Opaque,
            1 => NameEncoding::Utf8,
            2 => NameEncoding::WindowsUtf16Le,
            3 => NameEncoding::UnixBytes,
            _ => return Err(WireError::Unsupported),
        };
        let raw_name = self.blob()?;
        let display = self.text()?;
        let kind = match self.u8()? {
            1 => Kind::Directory,
            2 => Kind::File,
            3 => Kind::DirectoryLink,
            4 => Kind::Symlink,
            5 => Kind::Other,
            _ => return Err(WireError::Unsupported),
        };
        Ok(Node {
            id,
            address,
            name_encoding,
            raw_name,
            display,
            kind,
            metadata: self.metadata()?,
        })
    }
    fn finish(&self) -> Result<(), WireError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::Malformed)
        }
    }
}
