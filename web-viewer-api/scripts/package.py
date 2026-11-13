"""Build immutable offline public API/SDK releases using only Python's standard library."""

import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tarfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = ROOT.parent
REGISTRY = "https://midnight-explorer.invalid/api-index"
PACKAGES = ("midnight-web-viewer-api", "midnight-web-viewer-sdk")


def canonical_package(artifact, prefix):
    """Exclude unrelated Git commit metadata and preserve every source byte."""
    members = {}
    total = 0
    with tarfile.open(artifact, "r:gz") as archive:
        for member in archive:
            path = PurePosixPath(member.name)
            if (not member.isfile() or path.parts[0] != prefix
                    or len(path.parts) < 2 or ".." in path.parts
                    or "\\" in member.name or ":" in member.name
                    or member.size > 1024 * 1024 or len(members) >= 512
                    or member.name in members):
                raise SystemExit("Invalid Cargo release member")
            data = archive.extractfile(member).read()
            total += len(data)
            if total > 16 * 1024 * 1024:
                raise SystemExit("Cargo release exceeds package budget")
            if path.name != ".cargo_vcs_info.json":
                members[member.name] = (member.mode, data)
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w", format=tarfile.GNU_FORMAT) as archive:
        for name, (mode, data) in sorted(members.items()):
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            entry.mode = 0o755 if mode & 0o111 else 0o644
            archive.addfile(entry, io.BytesIO(data))
    compressed = io.BytesIO()
    # gzip.compress(mtime=0) delegates its header to zlib on some Python
    # versions, embedding the host OS. GzipFile writes the portable OS=255 header.
    with gzip.GzipFile(fileobj=compressed, mode="wb", filename="", mtime=0, compresslevel=9) as encoder:
        encoder.write(stream.getvalue())
    return compressed.getvalue(), members


def main():
    output = (Path(sys.argv[1]) if len(sys.argv) > 1 else
              REPOSITORY / "_build/html-viewer/sdk" / f"release-{time.time_ns()}").resolve()
    if not output.is_relative_to((REPOSITORY / "_build").resolve()):
        raise SystemExit("Release output must stay beneath the repository's _build")
    output.mkdir(parents=True, exist_ok=False)
    registry = output / "registry"
    (registry / "index").mkdir(parents=True)
    config = output / "config.toml"
    config.write_text(
        f'[registries.midnight]\nindex = "{REGISTRY}"\n'
        f'[source.midnight]\nregistry = "{REGISTRY}"\n'
        'replace-with = "web-viewer-releases"\n[source.web-viewer-releases]\n'
        f'local-registry = {json.dumps(str(registry))}\n', encoding="utf-8")
    environment = os.environ.copy()
    environment.setdefault("CARGO_HOME", str(REPOSITORY / ".tools/cargo"))
    environment.setdefault("RUSTUP_HOME", str(REPOSITORY / ".tools/rustup"))
    cargo = environment.get("CARGO", "cargo")
    target = REPOSITORY / "_build/html-viewer/api-target"
    artifacts = []
    for name in PACKAGES:
        subprocess.run([
            cargo, "package", "--manifest-path", str(ROOT / "Cargo.toml"), "-p", name,
            "--registry", "midnight", "--config", str(config), "--target-dir", str(target),
            "--no-verify", "--allow-dirty", "--offline",
        ], check=True, cwd=REPOSITORY, env=environment)
        package = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        version = package["workspace"]["package"]["version"]
        prefix = f"{name}-{version}"
        data, members = canonical_package(target / f"package/{prefix}.crate", prefix)
        manifest = tomllib.loads(members[f"{prefix}/Cargo.toml"][1].decode("utf-8"))
        dependencies = []
        for dependency, fields in manifest.get("dependencies", {}).items():
            if (any(key in fields for key in ("path", "git", "registry"))
                    or fields.get("registry-index") != REGISTRY):
                raise SystemExit("Release dependency must name the public registry")
            dependencies.append({"name": dependency, "req": fields["version"],
                                 "kind": "normal", "target": None, "registry": None,
                                 "features": fields.get("features", []),
                                 "optional": fields.get("optional", False),
                                 "default_features": fields.get("default-features", True)})
        checksum = hashlib.sha256(data).hexdigest()
        record = {"name": name, "vers": version, "deps": dependencies, "cksum": checksum,
                  "features": manifest.get("features", {}), "yanked": False,
                  "links": None, "rust_version": manifest["package"]["rust-version"]}
        index = registry / "index" / name[:2] / name[2:4]
        index.mkdir(parents=True, exist_ok=True)
        (index / name).write_text(json.dumps(record) + "\n", encoding="utf-8")
        (registry / f"{prefix}.crate").write_bytes(data)
        artifacts.append({"archive": f"{prefix}.crate", "sha256": checksum, "bytes": len(data)})
    (output / "artifacts.json").write_text(json.dumps(artifacts, indent=2) + "\n", encoding="utf-8")
    print(config)


if __name__ == "__main__":
    main()
