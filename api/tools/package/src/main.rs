use midnight_api_package::{
    bundle_configuration, canonical_package, crate_member, index_record, packaged_crate, run_cargo,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    ffi::OsString,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Local registry source name inside the viewer API/SDK bundle configuration.
const SOURCE: &str = "midnight-releases";

fn main() -> Result<()> {
    let root = std::path::absolute(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: midnight-api-package REPOSITORY")?,
    )?;
    let build = root.join("_build");
    fs::create_dir_all(&build)?;
    // Canonical paths validate containment only. Cargo's Windows packaging file
    // enumeration currently loses sources when its manifest uses a verbatim prefix.
    if !fs::canonicalize(&build)?.starts_with(fs::canonicalize(&root)?) {
        return Err("_build resolves outside the repository".into());
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let output = build.join(format!("plugin-sdk-{stamp}"));
    fs::create_dir(&output)?; // Fresh immutable bundle; never delete or overwrite another build.
    let registry = output.join("registry");
    fs::create_dir_all(registry.join("index"))?;
    fs::create_dir(output.join(".cargo"))?;
    let config = output.join(".cargo/config.toml");
    fs::write(&config, bundle_configuration(SOURCE, false))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let manifest = root.join("api/Cargo.toml");
    let mut records = Vec::new();
    for name in ["midnight-plugin-api", "midnight-plugin-sdk"] {
        run_cargo(
            &cargo,
            &root,
            &output,
            [
                OsString::from("package"),
                "--manifest-path".into(),
                manifest.clone().into_os_string(),
                "-p".into(),
                name.into(),
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
        let bytes = fs::read(&archive)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("API package exceeds distribution budget".into());
        }
        let filename = archive.file_name().ok_or("missing archive filename")?;
        let archive_name = filename.to_str().ok_or("non-Unicode Cargo package name")?;
        let prefix = archive_name.trim_end_matches(".crate");
        let cargo_checksum = format!("{:x}", Sha256::digest(&bytes));
        let provenance = crate_member(&bytes, &format!("{prefix}/.cargo_vcs_info.json"))?;
        let bytes = canonical_package(&bytes, prefix)?;
        let manifest_bytes = crate_member(&bytes, &format!("{prefix}/Cargo.toml"))?;
        let manifest: toml::Value = toml::from_str(std::str::from_utf8(&manifest_bytes)?)?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        let record = index_record(&manifest, &checksum)?;
        let index = registry.join("index").join(&name[..2]).join(&name[2..4]);
        fs::create_dir_all(&index)?;
        fs::write(index.join(name), format!("{record}\n"))?;
        fs::write(registry.join(filename), &bytes)?;
        if name == "midnight-plugin-api" {
            let include = output.join("include");
            fs::create_dir(&include)?;
            fs::write(
                include.join("midnight_plugin_v1.h"),
                crate_member(&bytes, &format!("{prefix}/include/midnight_plugin_v1.h"))?,
            )?;
        }
        let vcs: serde_json::Value = serde_json::from_slice(&provenance)?;
        records.push(json!({
            "archive": archive_name, "sha256": checksum, "bytes": bytes.len(),
            "cargo_archive_sha256": cargo_checksum, "vcs": vcs,
        }));
    }
    let vendor = output.join("vendor");
    run_cargo(
        &cargo,
        &root,
        &output,
        [
            OsString::from("vendor"),
            "--manifest-path".into(),
            manifest.into_os_string(),
            "--locked".into(),
            "--offline".into(),
            "--versioned-dirs".into(),
            vendor.clone().into_os_string(),
        ],
    )?;
    fs::write(&config, bundle_configuration(SOURCE, true))?;
    fs::write(
        output.join("artifacts.json"),
        serde_json::to_vec_pretty(&records)?,
    )?;
    println!("Public API/SDK release bundle: {}", output.display());
    Ok(())
}
