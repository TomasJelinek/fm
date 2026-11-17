//! Independent release bundle for new executable-worker APIs. Existing viewer releases are untouched.
use midnight_api_package::{
    bundle_configuration, canonical_package, crate_member, index_record, packaged_crate, run_cargo,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    ffi::OsString,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Local registry source name inside the worker API/SDK bundle configuration.
const SOURCE: &str = "worker-releases";

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let root = std::path::absolute(
        arguments
            .next()
            .ok_or("usage: worker-packages REPOSITORY BUILD_ROOT")?,
    )?;
    let build = std::path::absolute(arguments.next().ok_or("Missing explicit BUILD_ROOT")?)?;
    if !build
        .components()
        .any(|component| component.as_os_str() == "_build")
    {
        return Err("SDK output must be below _build".into());
    }
    fs::create_dir_all(&build)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let output = build.join(format!("worker-sdk-{stamp}"));
    fs::create_dir(&output)?;
    let registry = output.join("registry");
    fs::create_dir_all(registry.join("index"))?;
    fs::create_dir(output.join(".cargo"))?;
    let config = output.join(".cargo/config.toml");
    fs::write(&config, bundle_configuration(SOURCE, false))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    // Every package is required: archive, image and PDF plugins resolve their pinned
    // versions from this one bundle, so a missing workspace must not be skipped.
    let packages = [
        ("worker-api", "midnight-worker-sdk"),
        ("thumbnail-api", "midnight-thumbnail-api"),
        ("thumbnail-api", "midnight-thumbnail-sdk"),
        ("filesystem-api", "midnight-filesystem-api"),
        ("image-viewer-api", "midnight-image-viewer-api"),
        ("image-viewer-api", "midnight-image-viewer-sdk"),
    ];
    let mut artifacts = Vec::new();
    let mut workspaces = Vec::<PathBuf>::new();
    for (workspace, name) in packages {
        let manifest = root.join(workspace).join(name).join("Cargo.toml");
        if !manifest.is_file() {
            return Err(format!("Missing public package {name}").into());
        }
        let workspace_manifest = root.join(workspace).join("Cargo.toml");
        if !workspaces.contains(&workspace_manifest) {
            workspaces.push(workspace_manifest);
        }
        run_cargo(
            &cargo,
            &root,
            &output,
            [
                OsString::from("package"),
                "--manifest-path".into(),
                manifest.into_os_string(),
                "--registry".into(),
                "midnight".into(),
                "--config".into(),
                config.clone().into_os_string(),
                "--no-verify".into(),
                "--allow-dirty".into(),
                "--offline".into(),
            ],
        )?;
        let archive = packaged_crate(&output, name)?;
        let filename = archive.file_name().ok_or("Missing package name")?;
        let archive_name = filename.to_str().ok_or("Invalid package name")?;
        let prefix = archive_name.trim_end_matches(".crate");
        let original = fs::read(&archive)?;
        if original.len() > 16 * 1024 * 1024 {
            return Err("Worker API package exceeds budget".into());
        }
        let bytes = canonical_package(&original, prefix)?;
        let manifest = crate_member(&bytes, &format!("{prefix}/Cargo.toml"))?;
        let manifest: toml::Value = toml::from_str(std::str::from_utf8(&manifest)?)?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        let record = index_record(&manifest, &checksum)?;
        let index = registry.join("index").join(&name[..2]).join(&name[2..4]);
        fs::create_dir_all(&index)?;
        fs::write(index.join(name), format!("{record}\n"))?;
        fs::write(registry.join(filename), &bytes)?;
        artifacts.push(json!({"archive": archive_name, "sha256": checksum, "bytes": bytes.len()}));
    }
    let mut vendor_arguments: Vec<OsString> = vec![
        "vendor".into(),
        "--manifest-path".into(),
        workspaces[0].clone().into_os_string(),
        "--locked".into(),
        "--offline".into(),
        "--versioned-dirs".into(),
    ];
    for workspace in &workspaces[1..] {
        vendor_arguments.extend(["--sync".into(), workspace.clone().into_os_string()]);
    }
    vendor_arguments.push(output.join("vendor").into_os_string());
    run_cargo(&cargo, &root, &output, vendor_arguments)?;
    fs::write(&config, bundle_configuration(SOURCE, true))?;
    // Independent plugins may add their own crates.io dependencies. This registry-only config
    // selects our immutable API archives while allowing their own pinned vendor/cache policy.
    fs::write(
        output.join(".cargo/registry.toml"),
        bundle_configuration(SOURCE, false),
    )?;
    fs::write(
        output.join("artifacts.json"),
        serde_json::to_vec_pretty(&artifacts)?,
    )?;
    println!("Public worker API/SDK release bundle: {}", output.display());
    Ok(())
}
