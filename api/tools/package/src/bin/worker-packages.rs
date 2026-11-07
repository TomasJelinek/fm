//! Independent release bundle for new executable-worker APIs. Existing viewer releases are untouched.
use flate2::read::GzDecoder;
use midnight_api_package::{REGISTRY, canonical_package, index_record};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

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
    fs::write(&config, configuration(false))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let packages = [
        ("worker-api", "midnight-worker-sdk"),
        ("thumbnail-api", "midnight-thumbnail-api"),
        ("thumbnail-api", "midnight-thumbnail-sdk"),
        ("filesystem-api", "midnight-filesystem-api"),
    ];
    let mut artifacts = Vec::new();
    let mut workspaces = Vec::<PathBuf>::new();
    for (workspace, name) in packages {
        let manifest = root.join(workspace).join(name).join("Cargo.toml");
        if !manifest.is_file() {
            if workspace == "filesystem-api" {
                continue;
            }
            return Err(format!("Missing public package {name}").into());
        }
        let workspace_manifest = root.join(workspace).join("Cargo.toml");
        if !workspaces.contains(&workspace_manifest) {
            workspaces.push(workspace_manifest);
        }
        run(
            &cargo,
            &root,
            &output,
            vec![
                "package".into(),
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
        let archive = fs::read_dir(output.join("cargo/package"))?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|file| file.to_str())
                    .is_some_and(|file| {
                        file.starts_with(&format!("{name}-")) && file.ends_with(".crate")
                    })
            })
            .ok_or("Cargo did not emit the expected public worker package")?;
        let filename = archive.file_name().ok_or("Missing package name")?;
        let archive_name = filename.to_str().ok_or("Invalid package name")?;
        let prefix = archive_name.trim_end_matches(".crate");
        let original = fs::read(&archive)?;
        if original.len() > 16 * 1024 * 1024 {
            return Err("Worker API package exceeds budget".into());
        }
        let bytes = canonical_package(&original, prefix)?;
        let manifest = member(&bytes, &format!("{prefix}/Cargo.toml"))?;
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
    run(&cargo, &root, &output, vendor_arguments)?;
    fs::write(&config, configuration(true))?;
    // Independent plugins may add their own crates.io dependencies. This registry-only config
    // selects our immutable API archives while allowing their own pinned vendor/cache policy.
    fs::write(output.join(".cargo/registry.toml"), configuration(false))?;
    fs::write(
        output.join("artifacts.json"),
        serde_json::to_vec_pretty(&artifacts)?,
    )?;
    println!("Public worker API/SDK release bundle: {}", output.display());
    Ok(())
}

fn run(cargo: &OsString, root: &Path, output: &Path, arguments: Vec<OsString>) -> Result<()> {
    let status = Command::new(cargo)
        .current_dir(root)
        .args(arguments)
        .env("CARGO_TARGET_DIR", output.join("cargo"))
        .status()?;
    if !status.success() {
        return Err(format!("Worker API packaging failed: {status}").into());
    }
    Ok(())
}

fn configuration(vendor: bool) -> String {
    let mut config = format!(
        "[registries.midnight]\nindex = {REGISTRY:?}\n\n[source.midnight]\nregistry = {REGISTRY:?}\nreplace-with = 'worker-releases'\n\n[source.worker-releases]\nlocal-registry = 'registry'\n"
    );
    if vendor {
        config.push_str("\n[source.crates-io]\nreplace-with = 'third-party'\n\n[source.third-party]\ndirectory = 'vendor'\n");
    }
    config
}

fn member(archive: &[u8], name: &str) -> Result<Vec<u8>> {
    for entry in tar::Archive::new(GzDecoder::new(Cursor::new(archive))).entries()? {
        let entry = entry?;
        if entry.path()?.as_ref() != Path::new(name) {
            continue;
        }
        if !entry.header().entry_type().is_file() || entry.size() > 1024 * 1024 {
            return Err("Invalid worker package manifest".into());
        }
        let mut bytes = Vec::new();
        entry.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err("Oversized worker manifest".into());
        }
        return Ok(bytes);
    }
    Err(format!("Missing worker release member {name}").into())
}
