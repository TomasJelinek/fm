//! Filename hints prepared during backend listing, never evidence of a valid archive.
use std::path::Path;

/// Container families handled by independently installed filesystem providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    Gzip,
    TarGzip,
    SevenZip,
}

/// One configurable filename association. The extension is ASCII without a leading dot.
/// Enabling it changes panel activation only, never the system file association.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveAssociation {
    pub extension: &'static str,
    pub label: &'static str,
    pub format: ArchiveFormat,
    pub enabled_by_default: bool,
}

/// Recognize a documented container suffix without allocating or decoding native filenames.
/// This performs no I/O and does not check user preferences or installed providers.
/// The worker validates actual content when opening. See `docs/ARCHIVE_FORMATS.md`.
pub fn archive_association(path: &Path) -> Option<&'static ArchiveAssociation> {
    let name = path.file_name()?.as_encoded_bytes();
    // Match the compound suffix before ".gz". Slice bytes only for ASCII comparison.
    let extension = if name.len() >= 7 && name[name.len() - 7..].eq_ignore_ascii_case(b".tar.gz") {
        b"tar.gz".as_slice()
    } else {
        path.extension()?.as_encoded_bytes()
    };
    ARCHIVE_ASSOCIATIONS
        .iter()
        .find(|association| extension.eq_ignore_ascii_case(association.extension.as_bytes()))
}

/// Find the decoder family; panel activation separately honors saved associations.
pub fn archive_format(path: &Path) -> Option<ArchiveFormat> {
    archive_association(path).map(|association| association.format)
}

const fn association(
    extension: &'static str,
    label: &'static str,
    format: ArchiveFormat,
    enabled_by_default: bool,
) -> ArchiveAssociation {
    ArchiveAssociation {
        extension,
        label,
        format,
        enabled_by_default,
    }
}

/// Known containers, including opt-in document, media and application packages.
/// Defaults deliberately preserve native opening for end-user documents and installers.
pub const ARCHIVE_ASSOCIATIONS: &[ArchiveAssociation] = &[
    association("zip", "ZIP archive", ArchiveFormat::Zip, true),
    association("whl", "Python package", ArchiveFormat::Zip, true),
    association("egg", "Python package", ArchiveFormat::Zip, true),
    association("war", "Java deployment archive", ArchiveFormat::Zip, true),
    association("ear", "Java deployment archive", ArchiveFormat::Zip, true),
    association(
        "aab",
        "Android development package",
        ArchiveFormat::Zip,
        true,
    ),
    association(
        "aar",
        "Android development package",
        ArchiveFormat::Zip,
        true,
    ),
    association("nupkg", "NuGet package", ArchiveFormat::Zip, true),
    association("snupkg", "NuGet package", ArchiveFormat::Zip, true),
    association("jar", "Java application", ArchiveFormat::Zip, false),
    association("pyz", "Python application", ArchiveFormat::Zip, false),
    association("pyzw", "Python application", ArchiveFormat::Zip, false),
    association(
        "apk",
        "Android application installer",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "vsix",
        "Visual Studio / VS Code extension",
        ArchiveFormat::Zip,
        false,
    ),
    association("xpi", "Firefox extension", ArchiveFormat::Zip, false),
    association("epub", "E-book / comic", ArchiveFormat::Zip, false),
    association("cbz", "E-book / comic", ArchiveFormat::Zip, false),
    association("kmz", "Google Earth map", ArchiveFormat::Zip, false),
    association("kra", "Krita / OpenRaster image", ArchiveFormat::Zip, false),
    association("ora", "Krita / OpenRaster image", ArchiveFormat::Zip, false),
    association(
        "docx",
        "Word document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "docm",
        "Word document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "dotx",
        "Word document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "dotm",
        "Word document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "xlsx",
        "Excel workbook / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "xlsm",
        "Excel workbook / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "xltx",
        "Excel workbook / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "xltm",
        "Excel workbook / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "xlam",
        "Excel workbook / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "xlsb",
        "Excel workbook / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "pptx",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "pptm",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "potx",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "potm",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "ppsx",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "ppsm",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "ppam",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "sldx",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "sldm",
        "PowerPoint presentation / template / add-in",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "odt",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "ott",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "odm",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "otm",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "ods",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "ots",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "odg",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "otg",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "odp",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "otp",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "odf",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association(
        "odb",
        "OpenDocument document / template",
        ArchiveFormat::Zip,
        false,
    ),
    association("tar", "TAR archive", ArchiveFormat::Tar, true),
    association("gz", "GZIP stream", ArchiveFormat::Gzip, true),
    association("gzip", "GZIP stream", ArchiveFormat::Gzip, true),
    association("tgz", "Gzipped TAR archive", ArchiveFormat::TarGzip, true),
    association(
        "tar.gz",
        "Gzipped TAR archive",
        ArchiveFormat::TarGzip,
        true,
    ),
    association("7z", "7-Zip archive", ArchiveFormat::SevenZip, true),
];
