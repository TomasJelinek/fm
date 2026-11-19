//! The third-party library list of the About dialog, read from the dependency
//! inventory that ships with the sources so the two cannot drift apart.
use crate::{LicenseRow, MainWindow};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

const INVENTORY: &str = include_str!("../../../docs/third-party-inventory.csv");

/// Splits one CSV record; quoted fields may hold commas and doubled quotes.
fn fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    out.push(field);
    out
}

/// Name, version and license of every library, sorted and without repeats.
pub(crate) fn parse(inventory: &str) -> Vec<(String, String, String)> {
    let mut rows: Vec<_> = inventory
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = fields(line).into_iter();
            let (name, version) = (columns.next()?, columns.next()?);
            let license = columns.nth(1)?;
            (!name.is_empty()).then_some((name, version, license))
        })
        .collect();
    rows.sort_by_key(|(name, version, _)| (name.to_lowercase(), version.clone()));
    rows.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    rows
}

pub(crate) fn matching(
    rows: &[(String, String, String)],
    query: &str,
) -> Vec<(String, String, String)> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|(name, _, license)| {
            query.is_empty()
                || name.to_lowercase().contains(&query)
                || license.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

fn show(window: &MainWindow, rows: &[(String, String, String)], total: usize) {
    let model: Vec<LicenseRow> = rows
        .iter()
        .map(|(name, version, license)| LicenseRow {
            name: name.as_str().into(),
            version: version.as_str().into(),
            license: license.as_str().into(),
        })
        .collect();
    window.set_license_rows(ModelRc::from(Rc::new(VecModel::from(model))));
    window.set_license_summary(SharedString::from(if rows.len() == total {
        format!("{total} libraries")
    } else {
        format!("{} of {total} libraries", rows.len())
    }));
}

/// Directories that may hold `<name>-<version>/` license folders: the `licenses`
/// folder shipped beside the executable, then a developer's cargo sources.
fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(dir) = &exe_dir {
        roots.push(dir.join("licenses"));
    }
    let mut registries: Vec<PathBuf> = exe_dir
        .iter()
        .flat_map(|dir| dir.ancestors())
        .map(|dir| dir.join(".tools/docker/cargo/registry/src"))
        .collect();
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        registries.push(PathBuf::from(home).join("registry/src"));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        registries.push(PathBuf::from(home).join(".cargo/registry/src"));
    }
    for registry in registries {
        if let Ok(entries) = fs::read_dir(registry) {
            roots.extend(entries.flatten().map(|entry| entry.path()));
        }
    }
    roots
}

fn find(roots: &[PathBuf], name: &str, version: &str) -> Option<PathBuf> {
    let folder = format!("{name}-{version}");
    roots
        .iter()
        .map(|root| root.join(&folder))
        .find(|path| path.is_dir())
}

const MAX_TEXT_BYTES: usize = 256 * 1024;

fn is_notice(name: &str) -> bool {
    let name = name.to_lowercase();
    ["license", "licence", "copying", "copyright", "notice"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Every notice file of the folder, one level deep, in name order.
fn notice_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if !is_notice(&entry.file_name().to_string_lossy()) {
            continue;
        }
        if path.is_dir() {
            files.extend(
                fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|inner| inner.path())
                    .filter(|inner| inner.is_file()),
            );
        } else if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// `authors = ["A <a@b>", ...]` of a Cargo manifest, without a TOML parser.
fn manifest_authors(manifest: &str) -> Vec<String> {
    let Some(start) = manifest.find("authors") else {
        return Vec::new();
    };
    let rest = &manifest[start..];
    let Some(open) = rest.find('[') else {
        return Vec::new();
    };
    let end = rest[open..].find(']').map_or(rest.len(), |end| open + end);
    let mut authors = Vec::new();
    let mut quoted = rest[open..end].split('"');
    quoted.next();
    while let (Some(author), Some(_)) = (quoted.next(), quoted.next()) {
        authors.push(author.to_owned());
    }
    authors
}

pub(crate) struct Detail {
    pub(crate) authors: String,
    pub(crate) text: String,
}

/// The license text and authors of one library folder.
pub(crate) fn read_detail(dir: &Path) -> Detail {
    let mut text = String::new();
    let mut authors: Vec<String> = fs::read_to_string(dir.join("Cargo.toml"))
        .map(|manifest| manifest_authors(&manifest))
        .unwrap_or_default();
    for file in notice_files(dir) {
        let Ok(bytes) = fs::read(&file) else { continue };
        let content = String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)]).into_owned();
        for line in content.lines() {
            let line = line.trim();
            if line.to_lowercase().starts_with("copyright") && !authors.iter().any(|a| a == line) {
                authors.push(line.to_owned());
            }
        }
        if text.len() < MAX_TEXT_BYTES {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned());
            text.push_str(&format!("──── {} ────\n\n", name.unwrap_or_default()));
            text.push_str(content.trim_end());
        }
    }
    authors.truncate(24);
    Detail {
        authors: authors.join("\n"),
        text,
    }
}

/// Authors the release inventory (`THIRD-PARTY.json` beside the executable) records.
fn inventory_authors(name: &str, version: &str) -> Vec<String> {
    let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("THIRD-PARTY.json")))
    else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(&text) else {
        return Vec::new();
    };
    entries
        .iter()
        .find(|entry| entry["name"] == name && entry["version"] == version)
        .and_then(|entry| entry["authors"].as_array())
        .map(|authors| {
            authors
                .iter()
                .filter_map(|author| author.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn open_detail(window: &MainWindow, roots: &[PathBuf], name: &str, version: &str, license: &str) {
    let mut detail = find(roots, name, version).map(|dir| read_detail(&dir));
    if let Some(detail) = detail.as_mut() {
        let mut authors = inventory_authors(name, version);
        authors.extend(detail.authors.lines().map(str::to_owned));
        authors.dedup();
        detail.authors = authors.join("\n");
    }
    window.set_license_detail_title(format!("{name} {version}").into());
    window.set_license_detail_license(license.into());
    match detail {
        Some(detail) if !detail.text.is_empty() => {
            window.set_license_detail_authors(if detail.authors.is_empty() {
                "Not stated in the license files.".into()
            } else {
                detail.authors.into()
            });
            window.set_license_detail_text(detail.text.into());
        }
        _ => {
            window.set_license_detail_authors("".into());
            window.set_license_detail_text(
                format!(
                    "The license text of {name} {version} is not installed with this build. \
                     Release packages keep it under licenses/{name}-{version}/. The declared \
                     license is {license}."
                )
                .into(),
            );
        }
    }
    window.set_license_detail_open(true);
}

pub(crate) fn install(window: &MainWindow) {
    let rows = Rc::new(parse(INVENTORY));
    show(window, &rows, rows.len());
    let weak = window.as_weak();
    let all = rows.clone();
    window.on_license_filter(move |query| {
        if let Some(window) = weak.upgrade() {
            show(&window, &matching(&rows, &query), rows.len());
        }
    });
    let weak = window.as_weak();
    window.on_license_open(move |name, version| {
        let Some(window) = weak.upgrade() else { return };
        let license = all
            .iter()
            .find(|(n, v, _)| n == name.as_str() && v == version.as_str())
            .map(|(_, _, license)| license.as_str())
            .unwrap_or_default();
        open_detail(&window, &roots(), &name, &version, license);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_inventory_lists_every_library_with_a_license() {
        let rows = parse(INVENTORY);
        assert!(rows.len() > 400, "{} libraries", rows.len());
        assert!(rows.iter().all(|(name, version, license)| {
            !name.is_empty() && !version.is_empty() && !license.is_empty()
        }));
        assert!(rows.iter().any(|(name, _, _)| name == "slint"));
    }

    #[test]
    fn a_library_folder_yields_its_license_text_and_authors() {
        let root = std::env::temp_dir().join(format!("about-licenses-{}", std::process::id()));
        let dir = root.join("demo-1.2.3");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("LICENSE-MIT"), "MIT License\n\nCopyright (c) 2020 Ada Lovelace\n").unwrap();
        fs::write(dir.join("README.md"), "not a notice").unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nauthors = [\"Ada <ada@example.org>\",\n \"Grace\"]\n",
        )
        .unwrap();
        assert_eq!(find(&[root.clone()], "demo", "1.2.3"), Some(dir.clone()));
        assert_eq!(find(&[root.clone()], "demo", "9"), None);
        let detail = read_detail(&dir);
        assert!(detail.text.contains("──── LICENSE-MIT ────") && !detail.text.contains("not a notice"));
        assert_eq!(
            detail.authors.lines().collect::<Vec<_>>(),
            ["Ada <ada@example.org>", "Grace", "Copyright (c) 2020 Ada Lovelace"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quoted_fields_keep_their_commas_and_filters_match_name_or_license() {
        assert_eq!(fields(r#"a,"b,c","d""e""#), ["a", "b,c", r#"d"e"#]);
        let rows = vec![
            ("Serde".into(), "1".into(), "MIT OR Apache-2.0".into()),
            ("zlib".into(), "2".into(), "Zlib".into()),
        ];
        assert_eq!(matching(&rows, " ser ").len(), 1);
        assert_eq!(matching(&rows, "SERDE").len(), 1);
        assert_eq!(matching(&rows, "zlib").len(), 1);
        assert_eq!(matching(&rows, "mit").len(), 1);
        assert_eq!(matching(&rows, "").len(), 2);
    }
}
