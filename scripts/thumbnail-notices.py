#!/usr/bin/env python3
"""Collect pinned thumbnail dependency notices without downloading or compiling."""

import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
PLUGINS = ("midnight-thumbnail-image", "midnight-thumbnail-video")
SDK_PACKAGES = {"midnight-thumbnail-api", "midnight-thumbnail-sdk", "midnight-worker-sdk"}
MIDNIGHT_REGISTRY = "registry+https://midnight-explorer.invalid/api-index"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"

# Reuse the release packager's public collection and plain-file checks. Its
# application/theme packaging policy does not apply to these independent workers.
_spec = importlib.util.spec_from_file_location("thumbnail_release_files", ROOT / "scripts/package-release.py")
_release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_release)


def first_party(package, repository):
    source = package.get("source")
    if package["name"] in SDK_PACKAGES and source in (None, MIDNIGHT_REGISTRY):
        return True
    if source is not None:
        return False
    expected = {repository / "plugins" / name for name in (*PLUGINS, "image-decoder")}
    return Path(package["manifest_path"]).parent in expected


def add_file(plan, source, relative):
    source, relative = Path(source).absolute(), Path(relative)
    if relative.is_absolute() or not relative.parts:
        raise ValueError(f"Invalid notice destination: {relative}")
    for part in relative.parts:
        _release.portable_component(part)
    if source.resolve() != source:
        raise ValueError(f"Notice input must not traverse symlinks: {source}")
    with _release.plain_file(source) as stream:
        content = stream.read()
    if not content:
        raise ValueError(f"Required notice is empty: {source}")
    plan[relative] = content


def add_tree(plan, directory, relative):
    for source in _release.plain_tree(directory, maximum=65536):
        add_file(plan, source, relative / source.relative_to(directory))


def ffmpeg_sys_override(package, repository):
    """This reviewed published crate declares WTFPL but omits the license text."""
    if (package["name"], package["version"], package.get("source"), package.get("license")) != (
            "ffmpeg-sys-next", "9.0.1", CRATES_IO, "WTFPL"):
        return None
    directory = repository / "packaging/licenses/ffmpeg-sys-next-9.0.1"
    if not all((directory / name).is_file() for name in ("LICENSE.WTFPL", "SOURCE.md")):
        return None
    return directory


def notice_plan(plugin, metadata, repository):
    packages = metadata.get("packages", [])
    roots = [package for package in packages if package["name"] == plugin]
    if len(roots) != 1:
        raise ValueError(f"Expected exactly one metadata root for {plugin}")
    plan, inventory = {}, []
    shared = repository / "plugins/image-decoder/notices"
    if plugin == "midnight-thumbnail-image":
        # Native dav1d, bundled fonts and MPL parser source are not all described
        # by Cargo. Preserve the checked-in distribution notice superset too.
        add_tree(plan, shared, Path("licenses"))
    add_file(plan, shared / "rust/COPYRIGHT-library.html", Path("licenses/rust/COPYRIGHT-library.html"))
    features = {node["id"]: sorted(node.get("features", []))
                for node in (metadata.get("resolve") or {}).get("nodes", [])}
    identities = set()
    for package in sorted(packages, key=lambda entry: (entry["name"], entry["version"])):
        name = f'{package["name"]}-{package["version"]}'
        _release.portable_component(name)
        if name in identities:
            raise ValueError(f"Ambiguous dependency source: {name}")
        identities.add(name)
        owned = first_party(package, repository)
        license_name = package.get("license")
        if not owned and (not license_name or license_name.upper() in ("UNKNOWN", "UNLICENSED")):
            raise ValueError(f"Unknown dependency license: {name}")
        source, files = _release.license_files(package)
        if not files and not owned:
            override = ffmpeg_sys_override(package, repository)
            if override is None:
                raise ValueError(f"Missing dependency license files: {name}")
            add_tree(plan, override, Path("licenses") / name)
            # Retain the exact published declaration alongside the clearly
            # identified canonical license text; do not claim it was bundled.
            for filename in ("Cargo.toml", "Cargo.toml.orig", ".cargo_vcs_info.json"):
                if (source / filename).is_file():
                    add_file(plan, source / filename, Path("licenses") / name / filename)
        for path in files:
            add_file(plan, path, Path("licenses") / name / path.relative_to(source))
        inventory.append({**{key: package.get(key) for key in
                             ("name", "version", "license", "repository", "source")},
                          "features": features.get(package["id"], []), "project_owned": owned})
    plugin_root = Path(roots[0]["manifest_path"]).parent
    add_file(plan, plugin_root / "README.md", Path("README.md"))
    for filename in ("LICENSE", "LICENCE", "COPYING", "NOTICE", "THIRD-PARTY.md"):
        if (plugin_root / filename).is_file():
            add_file(plan, plugin_root / filename, Path(filename))
    plan[Path("THIRD-PARTY.json")] = (json.dumps(inventory, indent=2, ensure_ascii=False) + "\n").encode()
    plan[Path("THIRD-PARTY.txt")] = (
        "Pinned dependency declarations and resolved features are in THIRD-PARTY.json.\n"
        "Original license and copyright notices are retained under licenses/.\n"
        "The inventory conservatively includes build, development and other-target dependencies.\n"
        "Image notices also retain the shared decoder's native, font and source notices.\n"
        "The separate FFmpeg runtime package must supply its corresponding source and build options.\n"
        "This inventory records declarations; it does not replace the dependency license review.\n"
        "These notices do not assign a license to project-owned code.\n"
    ).encode()
    return plan


def generate_notices(metadata_by_plugin, output, *, repository=ROOT):
    """Validate both complete metadata inputs before replacing generated notices."""
    repository, output = Path(repository).absolute(), Path(output).absolute()
    build = repository / "_build"
    if output == build or not output.is_relative_to(build):
        raise ValueError("Generated thumbnail notices must be under repository _build/")
    if output.resolve() != output:
        raise ValueError("Generated thumbnail notices must not traverse symlinks")
    for parent in (output, *output.parents):
        if parent.exists():
            _release.plain_metadata(parent)
    if set(metadata_by_plugin) != set(PLUGINS):
        raise ValueError("Both image and video thumbnail metadata inputs are required")
    plans = {plugin: notice_plan(plugin, metadata_by_plugin[plugin], repository) for plugin in PLUGINS}
    for plugin in PLUGINS:
        target = output / plugin
        if target.exists() or target.is_symlink():
            _release.plain_tree(target, maximum=65536)
    output.mkdir(parents=True, exist_ok=True)
    for plugin, plan in plans.items():
        target = output / plugin
        if target.exists():
            shutil.rmtree(target)
        target.mkdir()
        for relative, content in sorted(plan.items()):
            destination = target / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(content)


def main(argv=None, *, repository=ROOT):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True, help="Released thumbnail SDK Cargo configuration")
    parser.add_argument("--output", type=Path, required=True, help="Notice staging directory under _build/")
    args = parser.parse_args(argv)
    config = args.config.resolve(strict=True)
    metadata = {}
    for plugin in PLUGINS:
        result = subprocess.run(
            ["cargo", "metadata", "--manifest-path", str(repository / "plugins" / plugin / "Cargo.toml"),
             "--config", str(config), "--format-version", "1", "--locked", "--offline"],
            cwd=repository, check=True, stdout=subprocess.PIPE, text=True)
        metadata[plugin] = json.loads(result.stdout)
    generate_notices(metadata, args.output, repository=repository)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
    except (OSError, ValueError) as error:
        print(f"Thumbnail notices: {error}", file=sys.stderr)
        sys.exit(2)
