use midnight_core::{ArchiveFormat, archive_format};
use midnight_filesystem_api::{Format, Protocol};
use midnight_plugin_host::{CapturedFile, validate_plugin_path};
use serde::Deserialize;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Package {
    pub id: String,
    pub name: Option<String>,
    pub directory: PathBuf,
    pub version: String,
    /// Provider promises deterministic member IDs for identical source, format and version.
    pub stable_entry_ids: bool,
    pub executable: PathBuf,
    pub protocol: Protocol,
    pub(crate) formats: Vec<Format>,
}
impl Package {
    /// Validated archive formats in declaration order, with duplicates removed.
    pub fn available_formats(&self) -> Vec<ArchiveFormat> {
        available_archive_formats(&self.formats)
    }
}
#[derive(Default, Debug)]
pub struct Catalog {
    pub packages: Vec<Package>,
    pub diagnostics: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    id: String,
    kind: String,
    version: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    stable_entry_ids: bool,
    entry_executable: String,
    target: String,
    protocol: u32,
    formats: Vec<String>,
}
fn available_archive_formats<'a>(
    declared: impl IntoIterator<Item = &'a Format>,
) -> Vec<ArchiveFormat> {
    let mut formats = Vec::new();
    for format in declared {
        let format = match format {
            Format::Zip => ArchiveFormat::Zip,
            Format::Tar => ArchiveFormat::Tar,
            Format::Gzip => ArchiveFormat::Gzip,
            Format::TarGzip => ArchiveFormat::TarGzip,
            Format::SevenZip => ArchiveFormat::SevenZip,
            Format::Xz => ArchiveFormat::Xz,
            Format::TarXz => ArchiveFormat::TarXz,
            Format::Bzip2 => ArchiveFormat::Bzip2,
            Format::TarBzip2 => ArchiveFormat::TarBzip2,
            Format::Zstd => ArchiveFormat::Zstd,
            Format::TarZstd => ArchiveFormat::TarZstd,
        };
        if !formats.contains(&format) {
            formats.push(format);
        }
    }
    formats
}
impl Catalog {
    pub fn discover(directory: &Path) -> io::Result<Self> {
        let mut result = Self::default();
        let directory = directory.canonicalize()?;
        let root = directory.join("filesystems");
        if !root.try_exists()? {
            return Ok(result);
        }
        if let Err(error) =
            validate_plugin_path(&directory).and_then(|()| validate_plugin_path(&root))
        {
            result.diagnostics.push(error.to_string());
            return Ok(result);
        }
        for (index, entry) in std::fs::read_dir(&root)?.enumerate() {
            if index >= 64 {
                return Err(io::Error::other(
                    "Filesystem package count limit exceeded; a partial scan cannot rule out duplicates",
                ));
            }
            let path = entry?.path();
            match package(&path) {
                Ok(Some(package)) => result.packages.push(package),
                Ok(None) => (),
                Err(error) => result
                    .diagnostics
                    .push(format!("{}: {error}", path.display())),
            }
        }
        let mut counts = HashMap::new();
        for p in &result.packages {
            *counts.entry(p.id.clone()).or_insert(0) += 1;
        }
        result.packages.retain(|p| {
            if counts[&p.id] == 1 {
                true
            } else {
                result
                    .diagnostics
                    .push(format!("Duplicate filesystem ID {}", p.id));
                false
            }
        });
        // The default worker for a format must not depend on file-system enumeration order.
        result.packages.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }
    pub fn choose(&self, path: &Path) -> Option<(&Package, Format)> {
        self.choose_provider(path, None)
    }
    /// Only formats advertised by trusted, compatible installed workers.
    pub fn available_formats(&self) -> Vec<ArchiveFormat> {
        available_archive_formats(self.packages.iter().flat_map(|package| &package.formats))
    }
    /// Choose this process's archive spool root now if an installed worker stages
    /// content, so the first archive mount neither walks the spool candidates nor waits
    /// for the test spool that checks one: on a busy disk, creating that file can wait
    /// seconds for the volume's journal. Blocking file-system work for a backend thread
    /// at startup. An error is diagnostic only: the next mount checks the roots again.
    pub fn prepare_staging(&self) -> io::Result<()> {
        if self
            .packages
            .iter()
            .any(|package| package.protocol.supports_staging())
        {
            crate::staging::Staging::prepare_private()
        } else {
            Ok(())
        }
    }
    pub fn choose_provider(
        &self,
        path: &Path,
        provider: Option<&str>,
    ) -> Option<(&Package, Format)> {
        self.choose_format(archive_format(path)?, provider)
    }

    /// Explicit host decoder selection still requires a trusted worker advertisement.
    pub fn choose_format(
        &self,
        format: ArchiveFormat,
        provider: Option<&str>,
    ) -> Option<(&Package, Format)> {
        let format = match format {
            ArchiveFormat::Zip => Format::Zip,
            ArchiveFormat::Tar => Format::Tar,
            ArchiveFormat::Gzip => Format::Gzip,
            ArchiveFormat::TarGzip => Format::TarGzip,
            ArchiveFormat::SevenZip => Format::SevenZip,
            ArchiveFormat::Xz => Format::Xz,
            ArchiveFormat::TarXz => Format::TarXz,
            ArchiveFormat::Bzip2 => Format::Bzip2,
            ArchiveFormat::TarBzip2 => Format::TarBzip2,
            ArchiveFormat::Zstd => Format::Zstd,
            ArchiveFormat::TarZstd => Format::TarZstd,
        };
        self.packages
            .iter()
            .find(|p| p.formats.contains(&format) && provider.is_none_or(|id| p.id == id))
            .map(|p| (p, format))
    }
}
fn package(path: &Path) -> io::Result<Option<Package>> {
    let check = |path: &Path| validate_plugin_path(path).map_err(io::Error::other);
    check(path)?;
    let manifest_path = path.join("plugin.toml");
    check(&manifest_path)?;
    let source = CapturedFile::capture(&manifest_path).map_err(io::Error::other)?;
    if source.length() > 16 * 1024 {
        return Err(io::Error::other("Filesystem manifest limit"));
    }
    let bytes = source.read_at(0, 16 * 1024).map_err(io::Error::other)?;
    let text = String::from_utf8(bytes).map_err(io::Error::other)?;
    let kind: toml::Value = toml::from_str(&text).map_err(io::Error::other)?;
    if matches!(
        kind.get("kind").and_then(toml::Value::as_str),
        Some("filesystem-native" | "filesystem-network")
    ) {
        // The native host owns validation/loading of its independently versioned ABI.
        return Ok(None);
    }
    let m: Manifest = toml::from_str(&text).map_err(io::Error::other)?;
    if m.schema_version != 1
        || !matches!(m.protocol, 1..=3)
        || m.kind != "filesystem-worker"
        || m.target != midnight_plugin_host::host_target()
        || m.version.is_empty()
        || m.version.len() > 128
        || m.name.as_ref().is_some_and(|name| {
            name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control)
        })
        || m.id.is_empty()
        || m.id.len() > 128
        || !m
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        || m.entry_executable.is_empty()
        || m.entry_executable.contains(['/', '\\', ':'])
        || m.entry_executable == "."
        || m.entry_executable == ".."
        || m.formats.len() > 16
    {
        return Err(io::Error::other("Incompatible filesystem manifest"));
    }
    let formats = m
        .formats
        .iter()
        .map(|f| match f.as_str() {
            "zip" => Ok(Format::Zip),
            "tar" => Ok(Format::Tar),
            "gzip" => Ok(Format::Gzip),
            "tar-gzip" => Ok(Format::TarGzip),
            "7z" => Ok(Format::SevenZip),
            "xz" => Ok(Format::Xz),
            "tar-xz" => Ok(Format::TarXz),
            "bzip2" => Ok(Format::Bzip2),
            "tar-bzip2" => Ok(Format::TarBzip2),
            "zstd" => Ok(Format::Zstd),
            "tar-zstd" => Ok(Format::TarZstd),
            _ => Err(io::Error::other("Unknown required archive format")),
        })
        .collect::<io::Result<Vec<_>>>()?;
    let executable = path.join(m.entry_executable);
    validate_package(path, &executable)?;
    CapturedFile::capture(&executable).map_err(io::Error::other)?;
    Ok(Some(Package {
        id: m.id,
        name: m.name,
        directory: path.to_owned(),
        version: m.version,
        stable_entry_ids: m.stable_entry_ids,
        protocol: Protocol::try_from(m.protocol as u16)?,
        executable,
        formats,
    }))
}

/// Trust the worker's package directory, entry executable and every other package object
/// (including dependent libraries the OS loader may resolve before worker containment).
/// Discovery checks this once; each mount repeats it before launching the executable.
pub(crate) fn validate_package(directory: &Path, executable: &Path) -> io::Result<()> {
    let check = |path: &Path| validate_plugin_path(path).map_err(io::Error::other);
    check(directory)?;
    check(executable)?;
    midnight_plugin_host::validate_plugin_package(directory).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_report_distinct_formats_in_first_declaration_order() {
        let mut catalog = catalog();
        catalog.packages[0].formats = vec![Format::Tar, Format::Zip, Format::Tar, Format::SevenZip];
        let mut second = catalog.packages[0].clone();
        second.id = "second".into();
        second.formats = vec![Format::Xz, Format::Zip, Format::TarXz, Format::Xz];
        let mut empty = second.clone();
        empty.id = "empty".into();
        empty.formats.clear();
        catalog.packages.extend([second, empty]);

        assert_eq!(
            catalog.packages[0].available_formats(),
            [
                ArchiveFormat::Tar,
                ArchiveFormat::Zip,
                ArchiveFormat::SevenZip,
            ]
        );
        assert_eq!(
            catalog.packages[1].available_formats(),
            [ArchiveFormat::Xz, ArchiveFormat::Zip, ArchiveFormat::TarXz,]
        );
        assert!(catalog.packages[2].available_formats().is_empty());
        assert_eq!(
            catalog.available_formats(),
            [
                ArchiveFormat::Tar,
                ArchiveFormat::Zip,
                ArchiveFormat::SevenZip,
                ArchiveFormat::Xz,
                ArchiveFormat::TarXz,
            ]
        );
    }

    #[test]
    fn package_and_catalog_formats_convert_every_supported_archive_format() {
        let mut catalog = catalog();
        catalog.packages[0].formats = vec![
            Format::TarZstd,
            Format::Zstd,
            Format::TarBzip2,
            Format::Bzip2,
            Format::TarXz,
            Format::Xz,
            Format::SevenZip,
            Format::TarGzip,
            Format::Gzip,
            Format::Tar,
            Format::Zip,
        ];
        let expected = [
            ArchiveFormat::TarZstd,
            ArchiveFormat::Zstd,
            ArchiveFormat::TarBzip2,
            ArchiveFormat::Bzip2,
            ArchiveFormat::TarXz,
            ArchiveFormat::Xz,
            ArchiveFormat::SevenZip,
            ArchiveFormat::TarGzip,
            ArchiveFormat::Gzip,
            ArchiveFormat::Tar,
            ArchiveFormat::Zip,
        ];
        assert_eq!(catalog.packages[0].available_formats(), expected);
        assert_eq!(catalog.available_formats(), expected);
        assert!(Catalog::default().available_formats().is_empty());
    }

    #[test]
    fn filesystem_friendly_name_metadata_is_optional_and_validated() {
        let legacy = format!(
            "schema_version=1\nid='archive.test'\nkind='filesystem-worker'\nversion='0.1.0'\nentry_executable='archive'\ntarget='{}'\nprotocol=2\nformats=['zip']\n",
            midnight_plugin_host::host_target()
        );
        assert!(toml::from_str::<Manifest>(&legacy).is_ok());
        assert!(
            toml::from_str::<Manifest>(&format!("{legacy}name='Archive filesystem'\n")).is_ok()
        );
    }

    #[test]
    fn older_manifests_do_not_grant_persistent_member_identity() {
        let manifest = r#"
schema_version = 1
id = "archive.fixture"
kind = "filesystem-worker"
version = "1.2.3"
entry_executable = "archive"
target = "fixture-target"
protocol = 2
formats = ["zip"]
"#;
        let old: Manifest = toml::from_str(manifest).unwrap();
        assert!(!old.stable_entry_ids);
        let advertised: Manifest =
            toml::from_str(&format!("{manifest}\nstable_entry_ids = true\n")).unwrap();
        assert!(advertised.stable_entry_ids);
    }

    #[test]
    fn zip_containers_route_to_installed_zip_provider() {
        let catalog = catalog();
        for extension in [
            "zip", "whl", "egg", "war", "ear", "aab", "aar", "nupkg", "snupkg", "jar", "pyz",
            "pyzw", "apk", "vsix", "xpi", "epub", "cbz", "kmz", "kra", "ora", "docx", "docm",
            "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "xlam", "xlsb", "pptx", "pptm", "potx",
            "potm", "ppsx", "ppsm", "ppam", "sldx", "sldm", "odt", "ott", "odm", "otm", "ods",
            "ots", "odg", "otg", "odp", "otp", "odf", "odb",
        ] {
            for extension in [extension.to_owned(), extension.to_ascii_uppercase()] {
                let path = PathBuf::from(format!("文档/žluťoučký.{extension}"));
                let (_, format) = catalog
                    .choose(&path)
                    .unwrap_or_else(|| panic!("{extension}"));
                assert_eq!(format, Format::Zip, "{extension}");
                assert!(Catalog::default().choose(&path).is_none());
                assert!(
                    catalog
                        .choose_provider(&path, Some("other-provider"))
                        .is_none()
                );
            }
        }
        // Filename routing is separate from the user's per-extension activation policy.
        for name in [
            "not.whl.exe",
            "file.doc",
            "file.xls",
            "file.ppt",
            "file.rar",
            "file.crx",
        ] {
            assert!(catalog.choose(Path::new(name)).is_none(), "{name}");
        }
        assert_eq!(
            catalog.choose(Path::new("package.TAR.GZ")).unwrap().1,
            Format::TarGzip
        );
    }

    #[test]
    fn availability_and_routing_require_the_specific_installed_format() {
        let mut catalog = catalog();
        catalog.packages[0].formats = vec![Format::Tar, Format::Tar];
        assert_eq!(catalog.available_formats(), [ArchiveFormat::Tar]);
        assert!(catalog.choose(Path::new("package.whl")).is_none());
        assert_eq!(
            catalog.choose(Path::new("package.tar")).unwrap().1,
            Format::Tar
        );
        assert_eq!(
            catalog
                .choose_format(ArchiveFormat::Tar, Some("fixture"))
                .unwrap()
                .1,
            Format::Tar
        );
        assert!(catalog.choose_format(ArchiveFormat::Zip, None).is_none());
        assert!(
            catalog
                .choose_format(ArchiveFormat::Tar, Some("other"))
                .is_none()
        );
        assert!(
            Catalog::default()
                .choose_format(ArchiveFormat::Tar, None)
                .is_none()
        );
    }

    #[test]
    fn compression_formats_route_only_to_advertised_worker_capabilities() {
        let mut catalog = catalog();
        for (extension, format) in [
            ("xz", Format::Xz),
            ("tar.xz", Format::TarXz),
            ("bz2", Format::Bzip2),
            ("tar.bz2", Format::TarBzip2),
            ("zst", Format::Zstd),
            ("tar.zst", Format::TarZstd),
        ] {
            let path = PathBuf::from(format!("archive.{extension}"));
            assert!(catalog.choose(&path).is_none());
            catalog.packages[0].formats = vec![format];
            assert_eq!(catalog.choose(&path).unwrap().1, format);
        }
    }

    pub(super) fn catalog() -> Catalog {
        Catalog {
            packages: vec![Package {
                name: None,
                directory: PathBuf::new(),
                protocol: Protocol::V1,
                id: "fixture".into(),
                version: "1".into(),
                stable_entry_ids: false,
                executable: PathBuf::new(),
                formats: vec![Format::Zip, Format::Tar, Format::TarGzip, Format::Gzip],
            }],
            diagnostics: vec![],
        }
    }

    #[cfg(target_os = "linux")]
    #[path = "tests_linux.rs"]
    mod linux;
    #[cfg(windows)]
    #[path = "tests_windows.rs"]
    mod windows;
}
