//! GitHub release listings and the choice of the release to offer.
use semver::Version;
use serde::Deserialize;

/// The most releases one listing holds; GitHub's page size is set to this.
pub const LISTED_RELEASES: usize = 30;

/// The platform this build updates: its release target and archive suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    /// The Rust target in archive names, such as `x86_64-pc-windows-msvc`.
    pub target: &'static str,
    /// `zip` or `tar.gz`.
    pub archive_suffix: &'static str,
}

/// The platform of this build, or `None` where no release archive is published.
pub const PLATFORM: Option<Platform> = platform();

const fn platform() -> Option<Platform> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some(Platform {
            target: "x86_64-pc-windows-msvc",
            archive_suffix: "zip",
        })
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        Some(Platform {
            target: "x86_64-unknown-linux-gnu",
            archive_suffix: "tar.gz",
        })
    } else {
        None
    }
}

impl Platform {
    /// The release archive's file name for `version`.
    pub fn archive_name(&self, version: &Version) -> String {
        format!(
            "midnight-explorer-{version}-{}.{}",
            self.target, self.archive_suffix
        )
    }

    /// The single folder at the root of the release archive.
    pub fn archive_root(&self, version: &Version) -> String {
        format!("midnight-explorer-{version}-{}", self.target)
    }
}

/// The checksum list every release carries, and its minisign signature.
pub const SUMS_NAME: &str = "SHA256SUMS";
pub const SIGNATURE_NAME: &str = "SHA256SUMS.minisig";

/// The version of this build.
pub fn current_version() -> Version {
    // The workspace version is valid semver; Cargo refuses anything else.
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version::new(0, 0, 0))
}

/// Parses a release tag: `v0.5.1`, `0.5.1` or `v0.6.0-beta.1`.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let text = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(text).ok()
}

/// One downloadable file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: String,
}

/// A published release, as far as the update check uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub title: String,
    pub notes: String,
    pub page_url: String,
    /// Marked as a pre-release on GitHub or carrying a semver pre-release part.
    pub preview: bool,
    pub published: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }

    /// Why this release cannot be installed automatically on `platform`, if it cannot.
    pub fn missing_files(&self, platform: &Platform) -> Option<String> {
        let archive = platform.archive_name(&self.version);
        let missing: Vec<&str> = [archive.as_str(), SUMS_NAME, SIGNATURE_NAME]
            .into_iter()
            .filter(|name| self.asset(name).is_none())
            .collect();
        if missing.is_empty() {
            None
        } else if missing == [SIGNATURE_NAME] {
            Some("This release is not signed, so it cannot be installed automatically.".into())
        } else {
            Some(format!(
                "This release has no {} for this system.",
                missing.join(", ")
            ))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ListingError {
    /// The body is not GitHub's release list.
    Malformed(String),
}

impl std::fmt::Display for ListingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(detail) => write!(f, "GitHub sent an unexpected answer ({detail})."),
        }
    }
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
}

/// Parses GitHub's release list. Drafts and tags that are not versions are left out.
pub fn parse_releases(body: &[u8]) -> Result<Vec<Release>, ListingError> {
    let raw: Vec<RawRelease> =
        serde_json::from_slice(body).map_err(|error| ListingError::Malformed(error.to_string()))?;
    Ok(raw
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = parse_tag(&release.tag_name)?;
            let preview = release.prerelease || !version.pre.is_empty();
            Some(Release {
                title: release
                    .name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| format!("Midnight Explorer {version}")),
                version,
                tag: release.tag_name,
                notes: release.body.unwrap_or_default(),
                page_url: release.html_url,
                preview,
                published: release.published_at.unwrap_or_default(),
                assets: release
                    .assets
                    .into_iter()
                    .map(|asset| Asset {
                        name: asset.name,
                        size: asset.size,
                        url: asset.browser_download_url,
                    })
                    .collect(),
            })
        })
        .collect())
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// No release is newer than this build.
    UpToDate,
    /// `release` is newer. `skipped` when the user chose to skip exactly this version.
    Newer { release: Release, skipped: bool },
}

/// Picks the newest release above `current`: previews only when `include_previews`.
pub fn newest(
    releases: &[Release],
    current: &Version,
    include_previews: bool,
    skipped: Option<&Version>,
) -> Finding {
    let best = releases
        .iter()
        .filter(|release| include_previews || !release.preview)
        .filter(|release| release.version > *current)
        .max_by(|a, b| a.version.cmp(&b.version));
    match best {
        None => Finding::UpToDate,
        Some(release) => Finding::Newer {
            skipped: skipped.is_some_and(|version| *version == release.version),
            release: release.clone(),
        },
    }
}
