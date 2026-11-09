//! Optional rich queries preserve raw OS text and independent capacity measurements.
use crate::{Cancel, NativeProvider, client::check_cancel, discovery::invalid, paths};
use midnight_filesystem_plugin_api::{Failure, NameEncoding, NativeText, Request, Response, abi};
use std::{ffi::OsString, path::Path};

impl NativeProvider {
    pub(crate) fn label(&self, path: &Path) -> Result<Option<OsString>, Failure> {
        self.require(abi::CAP_VOLUME_DETAILS)?;
        let Response::VolumeLabel(label) = self.exchange(
            Request::VolumeLabel {
                address: paths::encode(path)?,
            },
            &|| false,
        )?
        else {
            return Err(invalid("Missing volume label response"));
        };
        label.map(native_text).transpose()
    }

    pub(crate) fn information(
        &self,
        path: &Path,
        cancel: Cancel<'_>,
    ) -> Result<midnight_core::VolumeInformation, Failure> {
        check_cancel(cancel)?;
        self.require(abi::CAP_VOLUME_DETAILS)?;
        let Response::VolumeInformation(value) = self.exchange(
            Request::VolumeInformation {
                address: paths::encode(path)?,
            },
            cancel,
        )?
        else {
            return Err(invalid("Missing volume information response"));
        };
        check_cancel(cancel)?;
        Ok(midnight_core::VolumeInformation {
            root: value.root.as_ref().map(paths::decode).transpose()?,
            label: value.label.map(native_text).transpose()?,
            filesystem: value.filesystem.map(native_text).transpose()?,
            source: value.source.map(native_text).transpose()?,
            capacity: midnight_core::VolumeCapacity {
                total: value.capacity.total,
                free: value.capacity.free,
                available: value.capacity.available,
            },
            storage: storage_kind(value.storage),
            read_only: value.read_only,
            properties: value.properties.into_iter().map(property).collect(),
            unavailable: value.unavailable,
        })
    }
}

fn native_text(value: NativeText) -> Result<OsString, Failure> {
    if value.encoding == NameEncoding::Utf8 {
        if value.bytes.contains(&0) {
            return Err(invalid("NUL in volume text"));
        }
        String::from_utf8(value.bytes)
            .map(OsString::from)
            .map_err(|_| invalid("Invalid UTF-8 volume text"))
    } else {
        paths::name(value.encoding, &value.bytes)
    }
}

pub(crate) fn storage_kind(
    value: midnight_filesystem_plugin_api::StorageKind,
) -> midnight_core::StorageKind {
    use midnight_filesystem_plugin_api::StorageKind as Wire;
    match value {
        Wire::Unknown => midnight_core::StorageKind::Unknown,
        Wire::LocalSsd => midnight_core::StorageKind::LocalSsd,
        Wire::Rotational => midnight_core::StorageKind::Rotational,
        Wire::Network => midnight_core::StorageKind::Network,
    }
}

fn property(
    value: midnight_filesystem_plugin_api::VolumeProperty,
) -> midnight_core::VolumeProperty {
    use midnight_core::VolumeProperty as Core;
    use midnight_filesystem_plugin_api::VolumeProperty as Wire;
    match value {
        Wire::CasePreservedNames => Core::CasePreservedNames,
        Wire::CaseSensitiveLookup => Core::CaseSensitiveLookup,
        Wire::UnicodeNames => Core::UnicodeNames,
        Wire::PersistentAcls => Core::PersistentAcls,
        Wire::FileCompression => Core::FileCompression,
        Wire::SparseFiles => Core::SparseFiles,
        Wire::NamedStreams => Core::NamedStreams,
        Wire::HardLinks => Core::HardLinks,
        Wire::ReparsePoints => Core::ReparsePoints,
        Wire::Encryption => Core::Encryption,
        Wire::Quotas => Core::Quotas,
        Wire::NoExecute => Core::NoExecute,
        Wire::NoSetId => Core::NoSetId,
        Wire::NoDeviceFiles => Core::NoDeviceFiles,
    }
}
