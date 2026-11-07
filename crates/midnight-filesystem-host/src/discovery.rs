use midnight_core::{ArchiveFormat, archive_format};
use midnight_filesystem_api::Format;
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
    pub executable: PathBuf,
    formats: Vec<Format>,
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
    entry_executable: String,
    target: String,
    protocol: u32,
    formats: Vec<String>,
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
        for entry in std::fs::read_dir(&root)?.take(65) {
            if result.packages.len() + result.diagnostics.len() >= 64 {
                result
                    .diagnostics
                    .push("Filesystem package count limit".into());
                break;
            }
            let path = entry?.path();
            match package(&path) {
                Ok(package) => result.packages.push(package),
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
        Ok(result)
    }
    pub fn choose(&self, path: &Path) -> Option<(&Package, Format)> {
        self.choose_provider(path, None)
    }
    /// Only formats advertised by trusted, compatible installed workers.
    pub fn available_formats(&self) -> Vec<ArchiveFormat> {
        let mut formats = Vec::new();
        for format in self.packages.iter().flat_map(|package| &package.formats) {
            let format = match format {
                Format::Zip => ArchiveFormat::Zip,
                Format::Tar => ArchiveFormat::Tar,
                Format::Gzip => ArchiveFormat::Gzip,
                Format::TarGzip => ArchiveFormat::TarGzip,
                Format::SevenZip => ArchiveFormat::SevenZip,
            };
            if !formats.contains(&format) {
                formats.push(format);
            }
        }
        formats
    }
    pub fn choose_provider(
        &self,
        path: &Path,
        provider: Option<&str>,
    ) -> Option<(&Package, Format)> {
        let format = match archive_format(path)? {
            ArchiveFormat::Zip => Format::Zip,
            ArchiveFormat::Tar => Format::Tar,
            ArchiveFormat::Gzip => Format::Gzip,
            ArchiveFormat::TarGzip => Format::TarGzip,
            ArchiveFormat::SevenZip => Format::SevenZip,
        };
        self.packages
            .iter()
            .find(|p| p.formats.contains(&format) && provider.is_none_or(|id| p.id == id))
            .map(|p| (p, format))
    }
}
fn package(path: &Path) -> io::Result<Package> {
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
    let m: Manifest = toml::from_str(&text).map_err(io::Error::other)?;
    if m.schema_version != 1
        || m.protocol != 1
        || m.kind != "filesystem-worker"
        || m.target != midnight_plugin_host::host_target()
        || m.version.is_empty()
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
            _ => Err(io::Error::other("Unknown required archive format")),
        })
        .collect::<io::Result<Vec<_>>>()?;
    let executable = path.join(m.entry_executable);
    check(&executable)?;
    CapturedFile::capture(&executable).map_err(io::Error::other)?;
    Ok(Package {
        id: m.id,
        executable,
        formats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    pub(super) fn catalog() -> Catalog {
        Catalog {
            packages: vec![Package {
                id: "fixture".into(),
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
