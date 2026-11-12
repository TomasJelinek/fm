"""Create an offline public API directory source from Cargo's package artifact."""
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tarfile

root = pathlib.Path(__file__).resolve().parents[1]
out = pathlib.Path(sys.argv[1]).resolve()
if "_build" not in out.parts:
    raise SystemExit("SDK package output must be under _build")
name = sys.argv[3] if len(sys.argv) > 3 else "midnight-filesystem-api"
if len(sys.argv) > 2:
    root = pathlib.Path(sys.argv[2]).resolve()
subprocess.run(["cargo", "package", "--manifest-path", str(root / "Cargo.toml"),
                "-p", name, "--allow-dirty", "--no-verify", "--offline"], check=True)
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--manifest-path", str(root / "Cargo.toml"),
    "--format-version", "1", "--no-deps", "--offline"], text=True))
version = next(p["version"] for p in metadata["packages"] if p["name"] == name)
artifact = pathlib.Path(os.environ["CARGO_TARGET_DIR"]) / f"package/{name}-{version}.crate"
package = out / f"packages/{name}-{version}"
digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
if package.exists():
    previous = json.loads((package / ".cargo-checksum.json").read_text(encoding="utf-8"))
    if previous["package"] != digest:
        raise SystemExit("SDK artifact changed; use a new output bundle rather than overwrite immutable packages")
    print(out / "config.toml")
    raise SystemExit(0)
out.mkdir(parents=True, exist_ok=True)
with tarfile.open(artifact) as archive:
    archive.extractall(out / "packages", filter="data")
files = {str(p.relative_to(package)).replace("\\", "/"): hashlib.sha256(p.read_bytes()).hexdigest()
         for p in package.rglob("*") if p.is_file()}
(package / ".cargo-checksum.json").write_text(json.dumps({
    "files": files, "package": digest
}), encoding="utf-8")
(out / "config.toml").write_text(
    '[registries.midnight]\nindex = "https://midnight-explorer.invalid/api-index"\n'
    '[source.midnight]\nregistry = "https://midnight-explorer.invalid/api-index"\n'
    'replace-with = "filesystem-api-release"\n[source.filesystem-api-release]\ndirectory = '
    + json.dumps(str(out / "packages")) + '\n', encoding="utf-8")
print(out / "config.toml")
