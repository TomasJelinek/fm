use midnight_filesystem_plugin_api::{AddressEncoding, ErrorCode, Failure, abi};
use midnight_plugin_host::{CapturedFile, host_target, validate_plugin_path};
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Package {
    pub id: String,
    pub name: Option<String>,
    pub kind: PackageKind,
    pub version: String,
    pub directory: PathBuf,
    pub library: PathBuf,
    pub capabilities: u64,
    pub encoding: AddressEncoding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageKind {
    Native,
    Network,
}

#[derive(Debug, Default)]
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
    entry_library: String,
    target: String,
    abi_major: u16,
    minimum_host_minor: u16,
    protocol: u32,
    capabilities: Vec<String>,
    address_encoding: String,
}

impl Catalog {
    /// Discovery providers never qualify as native content providers.
    pub fn network_package(&self) -> Result<&Package, Failure> {
        let mut providers = self
            .packages
            .iter()
            .filter(|p| p.encoding == AddressEncoding::Opaque);
        let package = providers.next().ok_or_else(|| {
            Failure::new(
                ErrorCode::Unavailable,
                "Network Neighborhood plugin is unavailable",
            )
        })?;
        if providers.next().is_some() {
            return Err(invalid(
                "Several Network Neighborhood plugins claim this platform",
            ));
        }
        Ok(package)
    }
    /// Backend-only discovery beside the resolved installation executable.
    pub fn discover(directory: &Path) -> Result<Self, Failure> {
        let directory = directory.canonicalize().map_err(Failure::from_io)?;
        validate_plugin_path(&directory).map_err(invalid)?;
        let root = directory.join("filesystems");
        if !root.try_exists().map_err(Failure::from_io)? {
            return Ok(Self::default());
        }
        validate_plugin_path(&root).map_err(invalid)?;
        let mut catalog = Self::default();
        for (index, entry) in std::fs::read_dir(root)
            .map_err(Failure::from_io)?
            .enumerate()
        {
            if index >= 64 {
                return Err(Failure::new(
                    ErrorCode::ResourceLimit,
                    "Filesystem package count limit exceeded; a partial scan cannot rule out duplicate providers",
                ));
            }
            let path = entry.map_err(Failure::from_io)?.path();
            match read_package(&path) {
                Ok(Some(package)) => catalog.packages.push(package),
                Ok(None) => (),
                Err(error) => catalog
                    .diagnostics
                    .push(format!("{}: {error}", path.display())),
            }
        }
        let mut seen = HashSet::new();
        let mut duplicated = HashSet::new();
        for package in &catalog.packages {
            if !seen.insert(package.id.clone()) {
                duplicated.insert(package.id.clone());
            }
        }
        catalog
            .packages
            .retain(|package| !duplicated.contains(&package.id));
        for id in duplicated {
            catalog
                .diagnostics
                .push(format!("Duplicate native filesystem ID: {id}"));
        }
        catalog
            .packages
            .sort_by(|left, right| left.id.cmp(&right.id));
        Ok(catalog)
    }

    /// An absent or ambiguous provider is unavailable, never a request for fallback.
    pub fn native_package(&self) -> Result<&Package, Failure> {
        let mut compatible = self
            .packages
            .iter()
            .filter(|package| package.encoding == crate::paths::ENCODING);
        let first = compatible.next().ok_or_else(|| {
            Failure::new(
                ErrorCode::Unsupported,
                "Native filesystem plugin is unavailable",
            )
        })?;
        if compatible.next().is_some() {
            return Err(invalid(
                "Several native filesystem plugins claim this platform",
            ));
        }
        Ok(first)
    }
}

fn read_package(directory: &Path) -> Result<Option<Package>, Failure> {
    validate_plugin_path(directory).map_err(invalid)?;
    let path = directory.join("plugin.toml");
    validate_plugin_path(&path).map_err(invalid)?;
    let source = CapturedFile::capture(&path).map_err(invalid)?;
    if source.length() > 16 * 1024 {
        return Err(invalid("Filesystem manifest is too large"));
    }
    let bytes = source.read_at(0, 16 * 1024).map_err(invalid)?;
    let text = std::str::from_utf8(&bytes).map_err(invalid)?;
    let value: toml::Value = toml::from_str(text).map_err(invalid)?;
    if value.get("kind").and_then(toml::Value::as_str) == Some("filesystem-worker") {
        return Ok(None);
    }
    let package = parse_manifest(text, directory)?;
    validate_plugin_path(&package.library).map_err(invalid)?;
    CapturedFile::capture(&package.library).map_err(invalid)?;
    Ok(Some(package))
}

fn parse_manifest(text: &str, directory: &Path) -> Result<Package, Failure> {
    let manifest: Manifest = toml::from_str(text).map_err(invalid)?;
    if manifest.schema_version != 1
        || !matches!(
            manifest.kind.as_str(),
            "filesystem-native" | "filesystem-network"
        )
        || manifest.protocol != 1
        || manifest.abi_major != abi::ABI_MAJOR
        || manifest.minimum_host_minor > abi::ABI_MINOR
        || manifest.target != host_target()
        || manifest.id.is_empty()
        || manifest.id.len() > 128
        || !manifest
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
        || manifest.version.is_empty()
        || manifest.version.len() > 64
        || manifest.name.as_ref().is_some_and(|name| {
            name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control)
        })
        || manifest.entry_library.is_empty()
        || manifest.entry_library.contains(['/', '\\', ':', '\0'])
        || manifest.entry_library == "."
        || manifest.entry_library == ".."
    {
        return Err(invalid("Incompatible native filesystem manifest"));
    }
    let encoding = match manifest.address_encoding.as_str() {
        "opaque" => AddressEncoding::Opaque,
        "windows-utf16le" => AddressEncoding::WindowsUtf16Le,
        "unix-bytes" => AddressEncoding::UnixBytes,
        _ => return Err(invalid("Unsupported native address encoding")),
    };
    let network = manifest.kind == "filesystem-network";
    if (network && encoding != AddressEncoding::Opaque)
        || (!network && encoding != crate::paths::ENCODING)
    {
        return Err(invalid("Foreign native address encoding"));
    }
    let mut capabilities = 0;
    for name in manifest.capabilities {
        let bit = match name.as_str() {
            "browse" => abi::CAP_BROWSE,
            "read" => abi::CAP_READ,
            "stat" => abi::CAP_STAT,
            "volumes" => abi::CAP_VOLUMES,
            "storage" => abi::CAP_STORAGE,
            "volume-details" => abi::CAP_VOLUME_DETAILS,
            "native-address" => abi::CAP_NATIVE_ADDRESS,
            "activate" => abi::CAP_ACTIVATE,
            "refresh" => abi::CAP_REFRESH,
            "resolve-text" => abi::CAP_RESOLVE_TEXT,
            "prepared-rename" => abi::CAP_PREPARED_RENAME,
            _ => return Err(invalid("Unknown native filesystem capability")),
        };
        if capabilities & bit != 0 {
            return Err(invalid("Repeated native filesystem capability"));
        }
        capabilities |= bit;
    }
    let network_caps = abi::CAP_BROWSE
        | abi::CAP_STAT
        | abi::CAP_VOLUMES
        | abi::CAP_ACTIVATE
        | abi::CAP_REFRESH
        | abi::CAP_RESOLVE_TEXT;
    if network && (capabilities != network_caps || manifest.minimum_host_minor < 2) {
        return Err(invalid(
            "Network provider requires the discovery-only ABI 1.2 capabilities",
        ));
    }
    if !network && capabilities & abi::CAP_NATIVE_ADDRESS == 0 {
        return Err(invalid("Native address capability is required"));
    }
    if capabilities & abi::CAP_PREPARED_RENAME != 0 && manifest.minimum_host_minor < 3 {
        return Err(invalid("Prepared Rename requires ABI 1.3"));
    }
    Ok(Package {
        id: manifest.id,
        name: manifest.name,
        kind: if network {
            PackageKind::Network
        } else {
            PackageKind::Native
        },
        version: manifest.version,
        library: directory.join(manifest.entry_library),
        directory: directory.into(),
        capabilities,
        encoding,
    })
}

pub(crate) fn invalid(error: impl std::fmt::Display) -> Failure {
    Failure::new(ErrorCode::Invalid, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename_foundation_native_manifest(minimum_host_minor: u16, capabilities: &[&str]) -> String {
        format!(
            concat!(
                "schema_version=1\n",
                "id='native.rename-foundation'\n",
                "kind='filesystem-native'\n",
                "version='0.1.0'\n",
                "entry_library='native-test-library'\n",
                "target='{target}'\n",
                "abi_major={abi_major}\n",
                "minimum_host_minor={minimum_host_minor}\n",
                "protocol=1\n",
                "capabilities={capabilities:?}\n",
                "address_encoding='{encoding}'\n",
            ),
            target = host_target(),
            abi_major = abi::ABI_MAJOR,
            minimum_host_minor = minimum_host_minor,
            capabilities = capabilities,
            encoding = crate::paths::MANIFEST_ENCODING,
        )
    }

    fn rename_foundation_network_manifest(
        minimum_host_minor: u16,
        capabilities: &[&str],
    ) -> String {
        rename_foundation_native_manifest(minimum_host_minor, capabilities)
            .replace("kind='filesystem-native'", "kind='filesystem-network'")
            .replace(
                &format!("address_encoding='{}'", crate::paths::MANIFEST_ENCODING),
                "address_encoding='opaque'",
            )
    }

    #[test]
    fn rename_foundation_accepts_prepared_rename_for_current_native_platform() {
        for minimum_host_minor in 3..=abi::ABI_MINOR {
            let text = rename_foundation_native_manifest(
                minimum_host_minor,
                &["browse", "native-address", "prepared-rename"],
            );
            let package = parse_manifest(&text, Path::new("package"))
                .expect("ABI-compatible native prepared-rename manifest must be accepted");
            assert_eq!(package.kind, PackageKind::Native);
            assert_eq!(package.encoding, crate::paths::ENCODING);
            assert_eq!(
                package.capabilities,
                abi::CAP_BROWSE | abi::CAP_NATIVE_ADDRESS | abi::CAP_PREPARED_RENAME
            );
        }
    }

    #[test]
    fn rename_foundation_accepts_prepared_rename_without_unrelated_capabilities() {
        let text = rename_foundation_native_manifest(3, &["native-address", "prepared-rename"]);
        let package = parse_manifest(&text, Path::new("package"))
            .expect("native prepared-rename does not require unrelated Browse/Read capabilities");
        assert_eq!(package.kind, PackageKind::Native);
        assert_eq!(package.encoding, crate::paths::ENCODING);
        assert_eq!(
            package.capabilities,
            abi::CAP_NATIVE_ADDRESS | abi::CAP_PREPARED_RENAME
        );
    }

    #[test]
    fn rename_foundation_preserves_legacy_native_manifests_without_prepared_rename() {
        for minimum_host_minor in 0..=abi::ABI_MINOR {
            let text = rename_foundation_native_manifest(
                minimum_host_minor,
                &["browse", "native-address"],
            );
            let package = parse_manifest(&text, Path::new("package"))
                .expect("legacy native Browse manifests must remain accepted");
            assert_eq!(package.kind, PackageKind::Native);
            assert_eq!(package.encoding, crate::paths::ENCODING);
            assert_eq!(
                package.capabilities,
                abi::CAP_BROWSE | abi::CAP_NATIVE_ADDRESS
            );
        }
    }

    #[test]
    fn rename_foundation_rejects_prepared_rename_before_minor_three() {
        for minimum_host_minor in [0, 1, 2] {
            let text = rename_foundation_native_manifest(
                minimum_host_minor,
                &["browse", "native-address", "prepared-rename"],
            );
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(
                error.code,
                ErrorCode::Invalid,
                "prepared-rename must reject minimum_host_minor={minimum_host_minor}"
            );
        }
    }

    #[test]
    fn rename_foundation_rejects_prepared_rename_without_native_address() {
        for capabilities in [&["prepared-rename"][..], &["browse", "prepared-rename"][..]] {
            let text = rename_foundation_native_manifest(3, capabilities);
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{capabilities:?}");
        }
    }

    #[test]
    fn rename_foundation_rejects_prepared_rename_for_network_discovery() {
        for capabilities in [
            &[
                "browse",
                "stat",
                "volumes",
                "activate",
                "refresh",
                "resolve-text",
                "prepared-rename",
            ][..],
            &[
                "browse",
                "stat",
                "volumes",
                "activate",
                "refresh",
                "resolve-text",
                "native-address",
                "prepared-rename",
            ][..],
        ] {
            let text = rename_foundation_network_manifest(3, capabilities);
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{capabilities:?}");
        }
    }

    #[test]
    fn rename_foundation_rejects_prepared_rename_with_opaque_or_foreign_native_addresses() {
        let text =
            rename_foundation_native_manifest(3, &["browse", "native-address", "prepared-rename"]);
        for encoding in ["opaque", "windows-utf16le", "unix-bytes"] {
            if encoding == crate::paths::MANIFEST_ENCODING {
                continue;
            }
            let invalid_text = text.replace(
                &format!("address_encoding='{}'", crate::paths::MANIFEST_ENCODING),
                &format!("address_encoding='{encoding}'"),
            );
            let error = parse_manifest(&invalid_text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{encoding}");
        }
    }

    #[test]
    fn rename_foundation_rejects_duplicate_prepared_rename() {
        for capabilities in [
            &[
                "browse",
                "native-address",
                "prepared-rename",
                "prepared-rename",
            ][..],
            &[
                "prepared-rename",
                "browse",
                "prepared-rename",
                "native-address",
            ][..],
        ] {
            let text = rename_foundation_native_manifest(3, capabilities);
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{capabilities:?}");
        }
    }

    #[test]
    fn rename_foundation_rejects_unknown_capabilities_with_prepared_rename() {
        for unknown in ["unknown", "prepared_rename", "prepared-rename-extra"] {
            let text = rename_foundation_native_manifest(
                3,
                &["browse", "native-address", "prepared-rename", unknown],
            );
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{unknown}");
        }
    }

    #[test]
    fn rename_foundation_preserves_abi_and_target_validation() {
        for capabilities in [
            &["browse", "native-address"][..],
            &["browse", "native-address", "prepared-rename"][..],
        ] {
            let text = rename_foundation_native_manifest(3, capabilities);
            for invalid_text in [
                text.replace(
                    &format!("abi_major={}", abi::ABI_MAJOR),
                    &format!("abi_major={}", abi::ABI_MAJOR + 1),
                ),
                text.replace(
                    "minimum_host_minor=3",
                    &format!("minimum_host_minor={}", abi::ABI_MINOR + 1),
                ),
                text.replace(
                    &format!("target='{}'", host_target()),
                    "target='foreign-test-target'",
                ),
            ] {
                let error = parse_manifest(&invalid_text, Path::new("package")).unwrap_err();
                assert_eq!(error.code, ErrorCode::Invalid, "{invalid_text}");
            }
        }
    }

    #[test]
    fn rename_foundation_preserves_network_discovery_only_capabilities() {
        let discovery = [
            "browse",
            "stat",
            "volumes",
            "activate",
            "refresh",
            "resolve-text",
        ];
        let discovery_bits = abi::CAP_BROWSE
            | abi::CAP_STAT
            | abi::CAP_VOLUMES
            | abi::CAP_ACTIVATE
            | abi::CAP_REFRESH
            | abi::CAP_RESOLVE_TEXT;
        for minimum_host_minor in 2..=abi::ABI_MINOR {
            let text = rename_foundation_network_manifest(minimum_host_minor, &discovery);
            let package = parse_manifest(&text, Path::new("package"))
                .expect("existing discovery-only network manifests must remain accepted");
            assert_eq!(package.kind, PackageKind::Network);
            assert_eq!(package.encoding, AddressEncoding::Opaque);
            assert_eq!(package.capabilities, discovery_bits);
        }
        for minimum_host_minor in [0, 1] {
            let text = rename_foundation_network_manifest(minimum_host_minor, &discovery);
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "minor {minimum_host_minor}");
        }
        for missing in 0..discovery.len() {
            let mut capabilities = discovery.to_vec();
            capabilities.remove(missing);
            let text = rename_foundation_network_manifest(2, &capabilities);
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{capabilities:?}");
        }
        for extra in ["read", "storage", "volume-details", "native-address"] {
            let mut capabilities = discovery.to_vec();
            capabilities.push(extra);
            let text = rename_foundation_network_manifest(2, &capabilities);
            let error = parse_manifest(&text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{capabilities:?}");
        }
        let text = rename_foundation_network_manifest(2, &discovery);
        for encoding in ["windows-utf16le", "unix-bytes"] {
            let invalid_text = text.replace(
                "address_encoding='opaque'",
                &format!("address_encoding='{encoding}'"),
            );
            let error = parse_manifest(&invalid_text, Path::new("package")).unwrap_err();
            assert_eq!(error.code, ErrorCode::Invalid, "{encoding}");
        }
    }

    #[test]
    fn filesystem_friendly_name_metadata_is_optional_and_validated() {
        let legacy = format!(
            "schema_version=1\nid='network.test'\nkind='filesystem-network'\nversion='0.1.0'\nentry_library='network.dll'\ntarget='{}'\nabi_major=1\nminimum_host_minor=2\nprotocol=1\ncapabilities=['browse','stat','volumes','activate','refresh','resolve-text']\naddress_encoding='opaque'\n",
            host_target()
        );
        assert!(parse_manifest(&legacy, Path::new("package")).is_ok());
        assert!(
            parse_manifest(
                &format!("{legacy}name='Network filesystem'\n"),
                Path::new("package")
            )
            .is_ok()
        );
        for name in [
            String::new(),
            "   ".into(),
            "x".repeat(129),
            "bad\u{1b}name".into(),
        ] {
            assert!(
                parse_manifest(&format!("{legacy}name='{name}'\n"), Path::new("package")).is_err()
            );
        }
    }

    #[test]
    fn network_manifest_has_opaque_addresses_and_no_native_authority() {
        let text = format!(
            "schema_version=1\nid='network.test'\nkind='filesystem-network'\nversion='0.1.0'\nentry_library='network.dll'\ntarget='{}'\nabi_major=1\nminimum_host_minor=2\nprotocol=1\ncapabilities=['browse','stat','volumes','activate','refresh','resolve-text']\naddress_encoding='opaque'\n",
            host_target()
        );
        let package = parse_manifest(&text, Path::new("package")).unwrap();
        assert_eq!(package.encoding, AddressEncoding::Opaque);
        let catalog = Catalog {
            packages: vec![package],
            diagnostics: vec![],
        };
        assert!(catalog.native_package().is_err());
        for invalid_text in [
            text.replace("'opaque'", "'unix-bytes'"),
            text.replace("'resolve-text'", "'native-address'"),
            text.replace("'resolve-text'", "'read'"),
            text.replace("minimum_host_minor=2", "minimum_host_minor=1"),
            text.replace("'activate',", ""),
        ] {
            assert!(parse_manifest(&invalid_text, Path::new("package")).is_err());
        }
    }
    #[test]
    fn manifest_cannot_escape_package_or_invent_capabilities() {
        let text = format!(
            "schema_version = 1\nid = 'native.test'\nkind = 'filesystem-native'\nversion = '0.1.0'\nentry_library = 'native.dll'\ntarget = '{}'\nabi_major = 1\nminimum_host_minor = 0\nprotocol = 1\ncapabilities = ['browse', 'native-address']\naddress_encoding = '{}'\n",
            host_target(),
            crate::paths::MANIFEST_ENCODING
        );
        assert!(parse_manifest(&text, Path::new("package")).is_ok());
        for name in [
            "../native.dll",
            "C:drive.dll",
            "a/b.dll",
            "a\\b.dll",
            ".",
            "..",
            "",
        ] {
            assert!(
                parse_manifest(&text.replace("native.dll", name), Path::new("package")).is_err()
            );
        }
        for caps in ["'magic'", "'browse', 'browse'", "'browse'"] {
            assert!(
                parse_manifest(
                    &text.replace("'browse', 'native-address'", caps),
                    Path::new("package")
                )
                .is_err()
            );
        }
        assert!(parse_manifest(&format!("{text}extra = 1\n"), Path::new("package")).is_err());
        assert!(Catalog::default().native_package().is_err());
    }
}
