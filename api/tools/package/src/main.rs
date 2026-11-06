use flate2::read::GzDecoder;
use midnight_api_package::{REGISTRY, canonical_package, index_record};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

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
    fs::write(&config, configuration(false))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let manifest = root.join("api/Cargo.toml");
    let mut records = Vec::new();
    for name in ["midnight-plugin-api", "midnight-plugin-sdk"] {
        run(
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
        let packages = output.join("cargo/package");
        let archive = fs::read_dir(&packages)?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|file| {
                        file.starts_with(&format!("{name}-")) && file.ends_with(".crate")
                    })
            })
            .ok_or("Cargo did not produce the expected package")?;
        let bytes = fs::read(&archive)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("API package exceeds distribution budget".into());
        }
        let filename = archive.file_name().ok_or("missing archive filename")?;
        let archive_name = filename.to_str().ok_or("non-Unicode Cargo package name")?;
        let prefix = archive_name.trim_end_matches(".crate");
        let cargo_checksum = format!("{:x}", Sha256::digest(&bytes));
        let provenance = member(&bytes, &format!("{prefix}/.cargo_vcs_info.json"))?;
        let bytes = canonical_package(&bytes, prefix)?;
        let manifest_bytes = member(&bytes, &format!("{prefix}/Cargo.toml"))?;
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
                member(&bytes, &format!("{prefix}/include/midnight_plugin_v1.h"))?,
            )?;
        }
        let vcs: serde_json::Value = serde_json::from_slice(&provenance)?;
        records.push(json!({
            "archive": archive_name, "sha256": checksum, "bytes": bytes.len(),
            "cargo_archive_sha256": cargo_checksum, "vcs": vcs,
        }));
    }
    let vendor = output.join("vendor");
    run(
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
    fs::write(&config, configuration(true))?;
    fs::write(
        output.join("artifacts.json"),
        serde_json::to_vec_pretty(&records)?,
    )?;
    println!("Public API/SDK release bundle: {}", output.display());
    Ok(())
}

fn run<const N: usize>(
    cargo: &OsString,
    root: &Path,
    output: &Path,
    args: [OsString; N],
) -> Result<()> {
    let status = Command::new(cargo)
        .current_dir(root)
        .args(args)
        .env("CARGO_TARGET_DIR", output.join("cargo"))
        .status()?;
    if !status.success() {
        return Err(format!("Cargo packaging command failed: {status}").into());
    }
    Ok(())
}

fn configuration(include_vendor: bool) -> String {
    let mut config = format!(
        concat!(
            "[registries.midnight]\nindex = {registry:?}\n\n",
            "[source.midnight]\nregistry = {registry:?}\n",
            "replace-with = 'midnight-releases'\n\n",
            "[source.midnight-releases]\nlocal-registry = 'registry'\n",
        ),
        registry = REGISTRY,
    );
    if include_vendor {
        config.push_str(concat!(
            "\n[source.crates-io]\nreplace-with = 'third-party'\n\n",
            "[source.third-party]\ndirectory = 'vendor'\n",
        ));
    }
    config
}

fn member(archive: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(GzDecoder::new(Cursor::new(archive)));
    for entry in tar.entries()? {
        let entry = entry?;
        if entry.path()?.as_ref() != Path::new(name) {
            continue;
        }
        if !entry.header().entry_type().is_file() || entry.size() > 1024 * 1024 {
            return Err("invalid release metadata member".into());
        }
        let mut bytes = Vec::new();
        entry.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err("oversized release metadata".into());
        }
        return Ok(bytes);
    }
    Err(format!("missing release member {name}").into())
}
