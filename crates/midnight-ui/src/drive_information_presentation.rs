//! Presentation of measured native fields, with exact bytes alongside readable values.
use crate::format_size;
use midnight_core::{NumberLocale, StorageKind, VolumeInformation, VolumeProperty};
use std::path::Path;

pub(crate) fn body(
    context: &str,
    path: &Path,
    info: &VolumeInformation,
    locale: &NumberLocale,
) -> String {
    let mut lines = vec![format!("{context}: {}", path.display())];
    lines.push(format!(
        "Volume / mount: {}",
        info.root
            .as_ref()
            .map_or_else(|| "Unavailable".into(), |root| root.display().to_string())
    ));
    for (label, value) in [
        ("Volume label", &info.label),
        ("Filesystem", &info.filesystem),
        ("Native source / ID", &info.source),
    ] {
        lines.push(format!(
            "{label}: {}",
            value.as_ref().map_or_else(
                || "Unavailable".into(),
                |value| value.to_string_lossy().into_owned()
            )
        ));
    }
    lines.push(String::new());
    for (label, bytes) in [
        ("Total", info.capacity.total),
        ("Used", info.capacity.used()),
        ("Free on filesystem", info.capacity.free),
        ("Available to you", info.capacity.available),
    ] {
        lines.push(match bytes {
            Some(bytes) => format!(
                "{label}: {} bytes ({})",
                locale.format_u64(bytes),
                format_size(bytes, locale)
            ),
            None => format!("{label}: Unavailable"),
        });
    }
    lines.push(String::new());
    lines.push(format!(
        "Storage: {}",
        match info.storage {
            StorageKind::LocalSsd => "Local solid-state storage",
            StorageKind::Rotational => "Rotational storage",
            StorageKind::Network => "Network filesystem",
            StorageKind::Unknown => "Unavailable",
        }
    ));
    if let Some(read_only) = info.read_only {
        lines.push(format!(
            "Read-only filesystem: {}",
            if read_only { "Yes" } else { "No" }
        ));
    }
    if !info.properties.is_empty() {
        lines.push("Reported filesystem properties:".into());
        for property in &info.properties {
            lines.push(format!("  • {}", property_label(*property)));
        }
    }
    lines.push("\nSpace values are live measurements. Available space can differ because of quotas or filesystem reservations.".into());
    if !info.unavailable.is_empty() {
        lines.push("\nSome native details could not be read:".into());
        lines.extend(info.unavailable.iter().map(|error| format!("  • {error}")));
    }
    lines.join("\n")
}

fn property_label(property: VolumeProperty) -> &'static str {
    match property {
        VolumeProperty::CasePreservedNames => "Preserves filename case",
        VolumeProperty::CaseSensitiveLookup => "Supports case-sensitive lookup",
        VolumeProperty::UnicodeNames => "Unicode filenames",
        VolumeProperty::PersistentAcls => "Persistent access-control lists",
        VolumeProperty::FileCompression => "Per-file compression",
        VolumeProperty::SparseFiles => "Sparse files",
        VolumeProperty::NamedStreams => "Named data streams",
        VolumeProperty::HardLinks => "Hard links",
        VolumeProperty::ReparsePoints => "Reparse points",
        VolumeProperty::Encryption => "Filesystem encryption",
        VolumeProperty::Quotas => "Filesystem quotas",
        VolumeProperty::NoExecute => "Execution disabled by mount",
        VolumeProperty::NoSetId => "Set-user/group-ID disabled by mount",
        VolumeProperty::NoDeviceFiles => "Device-file access disabled by mount",
    }
}
