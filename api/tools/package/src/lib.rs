use serde_json::{Value, json};
mod archive;
pub use archive::canonical_package;

pub const REGISTRY: &str = "https://midnight-explorer.invalid/api-index";
pub const CRATES_IO: &str = "https://github.com/rust-lang/crates.io-index";

/// Convert Cargo's normalized release manifest, never a live path dependency,
/// into the registry-index schema. All package checksums describe actual .crate bytes.
pub fn index_record(manifest: &toml::Value, checksum: &str) -> Result<Value, String> {
    if checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid SHA-256 checksum".into());
    }
    let package = manifest.get("package").ok_or("missing package metadata")?;
    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .ok_or("missing name")?;
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("invalid registry package name".into());
    }
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or("missing version")?;
    let mut dependencies = Vec::new();
    add_dependencies(manifest, None, &mut dependencies)?;
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, fields) in targets {
            add_dependencies(fields, Some(target), &mut dependencies)?;
        }
    }
    let features = optional_json(manifest.get("features"), json!({}))?;
    Ok(json!({
        "name": name, "vers": version, "deps": dependencies, "cksum": checksum,
        "features": features,
        "yanked": false, "links": package.get("links"), "rust_version": package.get("rust-version"),
    }))
}

fn add_dependencies(
    manifest: &toml::Value,
    target: Option<&str>,
    output: &mut Vec<Value>,
) -> Result<(), String> {
    for (table, kind) in [
        ("dependencies", "normal"),
        ("build-dependencies", "build"),
        ("dev-dependencies", "dev"),
    ] {
        let Some(entries) = manifest.get(table).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, dependency) in entries {
            let fields = dependency
                .as_table()
                .ok_or("release dependency must be normalized")?;
            if fields.contains_key("path")
                || fields.contains_key("git")
                || fields.contains_key("registry")
            {
                return Err("unresolved source dependency in release manifest".into());
            }
            let version = fields
                .get("version")
                .and_then(toml::Value::as_str)
                .ok_or("missing dependency version")?;
            let registry = fields
                .get("registry-index")
                .and_then(toml::Value::as_str)
                .unwrap_or(CRATES_IO);
            let features = optional_json(fields.get("features"), json!([]))?;
            let optional = fields
                .get("optional")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false);
            let defaults = fields
                .get("default-features")
                .and_then(toml::Value::as_bool)
                .unwrap_or(true);
            output.push(json!({
                "name": name, "req": version, "kind": kind, "target": target,
                "registry": if registry == REGISTRY { None } else { Some(registry) },
                "features": features, "optional": optional, "default_features": defaults,
                "package": fields.get("package"),
            }));
        }
    }
    Ok(())
}

fn optional_json(value: Option<&toml::Value>, fallback: Value) -> Result<Value, String> {
    value
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| e.to_string())
        .map(|value| value.unwrap_or(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_dependencies_preserve_source_identity_and_strip_local_paths() {
        let manifest: toml::Value = toml::from_str(
            r#"
[package]
name = "midnight-plugin-sdk"
version = "0.1.0"
rust-version = "1.99"
[dependencies.midnight-plugin-api]
version = "=0.1.0"
registry-index = "https://midnight-explorer.invalid/api-index"
[dependencies.minicbor]
version = "=2.3.0"
features = ["std", "derive"]
default-features = false
"#,
        )
        .unwrap();
        let record = index_record(&manifest, "ab".repeat(32).as_str()).unwrap();
        assert_eq!(record["name"], "midnight-plugin-sdk");
        let deps = record["deps"].as_array().unwrap();
        let api = deps
            .iter()
            .find(|d| d["name"] == "midnight-plugin-api")
            .unwrap();
        assert!(api["registry"].is_null());
        let external = deps.iter().find(|d| d["name"] == "minicbor").unwrap();
        assert_eq!(
            external["registry"],
            "https://github.com/rust-lang/crates.io-index"
        );
        assert_eq!(external["default_features"], false);
        assert!(record.to_string().find("path").is_none());
    }

    #[test]
    fn malformed_or_local_dependency_manifests_cannot_enter_release_registry() {
        for dependency in [
            "path = '../host'",
            "git = 'https://example.invalid/private'",
        ] {
            let manifest: toml::Value = toml::from_str(&format!(
                "[package]\nname='sample'\nversion='0.1.0'\n[dependencies.bad]\nversion='1'\n{dependency}"
            )).unwrap();
            assert!(index_record(&manifest, &"ab".repeat(32)).is_err());
        }
    }
}
