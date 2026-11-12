#!/usr/bin/env python3
"""Generate dependency documentation from locked, target-filtered Cargo metadata.

Consumes metadata only: it does not download, build, change dependencies or decide
license compatibility. Capture instructions are in docs/THIRD_PARTY.md.
"""
import argparse
import csv
import importlib.util
import io
import json
from pathlib import Path
from urllib.parse import quote


ROOT = Path(__file__).resolve().parent.parent
SCOPES = {
    "app": "Cargo.toml",
    "renderer-tests": "vendor/femtovg/Cargo.toml",
    "api": "api/Cargo.toml",
    "basic": "plugins/midnight-viewer-basic/Cargo.toml",
    "source": "plugins/midnight-viewer-source/Cargo.toml",
    "markdown": "plugins/midnight-viewer-markdown/Cargo.toml",
    "conformance": "plugins/viewer-conformance/Cargo.toml",
    "filesystem-api": "filesystem-api/Cargo.toml",
    "native-windows": "plugins/midnight-filesystem-native-windows/Cargo.toml",
    "native-linux": "plugins/midnight-filesystem-native-linux/Cargo.toml",
    "native-macos": "plugins/midnight-filesystem-native-macos/Cargo.toml",
    "network": "plugins/midnight-filesystem-network/Cargo.toml",
    "worker-api": "worker-api/Cargo.toml",
    "thumbnail-api": "thumbnail-api/Cargo.toml",
    "image-viewer-api": "image-viewer-api/Cargo.toml",
    "web-viewer-api": "web-viewer-api/Cargo.toml",
    "html-viewer": "plugins/midnight-viewer-html/Cargo.toml",
    "image-viewer": "plugins/midnight-viewer-image/Cargo.toml",
    "image-decoder": "plugins/image-decoder/Cargo.toml",
    "pdf-viewer": "plugins/midnight-viewer-pdf/Cargo.toml",
    "archive": "plugins/midnight-fs-archive/Cargo.toml",
    "thumbnail-image": "plugins/midnight-thumbnail-image/Cargo.toml",
    "thumbnail-video": "plugins/midnight-thumbnail-video/Cargo.toml",
}
TARGETS = {"x86_64-pc-windows-msvc": "Windows", "x86_64-unknown-linux-gnu": "Linux"}
SCOPE_TARGETS = {
    "native-macos": {"x86_64-apple-darwin": "macOS x86-64",
                     "aarch64-apple-darwin": "macOS arm64"},
}


def vendored(package):
    marker = (package.get("metadata") or {}).get("midnight")
    return isinstance(marker, dict) and marker.get("vendored-third-party") is True


def identity(package):
    return package["name"], package["version"]


def inventory(captures, selections=None):
    owned = set()
    for metadata in captures.values():
        packages = {item["id"]: item for item in metadata["packages"]}
        for member in metadata["workspace_members"]:
            if member not in packages:
                raise ValueError(f"Missing workspace member: {member}")
            if not vendored(packages[member]):
                owned.add(identity(packages[member]))
    records = {}
    for (scope, target), metadata in sorted(captures.items()):
        packages = {item["id"]: item for item in metadata["packages"]}
        nodes = {item["id"]: item for item in metadata["resolve"]["nodes"]}
        pending = list(metadata["workspace_members"])
        seen = set()
        direct = {}
        if selections is not None:
            members = {identity(packages[key]) for key in metadata["workspace_members"]}
            missing = members - selections[scope, target]
            if missing:
                raise ValueError(f"Cargo tree omits workspace members in {scope}/{target}: {sorted(missing)}")
        for member in metadata["workspace_members"]:
            if member not in nodes:
                raise ValueError(f"Missing resolve node: {member}")
            if vendored(packages[member]):
                continue
            for edge in nodes[member]["deps"]:
                direct.setdefault(edge["pkg"], set()).add(packages[member]["name"])
        while pending:
            key = pending.pop()
            if key in seen:
                continue
            seen.add(key)
            if key not in packages or key not in nodes:
                raise ValueError(f"Incomplete resolve graph: {key}")
            package = packages[key]
            if selections is not None and identity(package) not in selections[scope, target]:
                continue
            pending.extend(edge["pkg"] for edge in nodes[key]["deps"])
            if identity(package) in owned:
                continue
            source = package.get("source") or "local path dependency"
            record_key = (*identity(package), source)
            license_text = package.get("license") or "UNKNOWN"
            if record_key in records and records[record_key]["license"] != license_text:
                raise ValueError(f"Conflicting license metadata for {record_key}")
            row = records.setdefault(record_key, {
                "name": package["name"], "version": package["version"],
                "license": license_text, "source": source,
                "scopes": set(), "targets": set(), "direct_by": set(),
            })
            row["scopes"].add(scope)
            row["targets"].add(target)
            row["direct_by"].update(direct.get(key, ()))
        if selections is not None:
            reached = {identity(packages[key]) for key in seen}
            missing = selections[scope, target] - reached
            if missing:
                raise ValueError(f"Unreachable Cargo tree packages in {scope}/{target}: {sorted(missing)}")
    rows = []
    for _, row in sorted(records.items()):
        rows.append({key: sorted(value) if isinstance(value, set) else value
                     for key, value in row.items()})
    return rows


def cell(value):
    return str(value).replace("&", "&amp;").replace("<", "&lt;").replace(
        ">", "&gt;").replace("|", "&#124;").replace("\n", " ").replace("`", "&#96;")


def library(row):
    name = cell(row["name"])
    if row["source"].startswith("registry+"):
        url = f'https://crates.io/crates/{quote(row["name"], safe="")}/{quote(row["version"], safe="")}'
        return f"[{name}]({url})"
    return name


def render(rows):
    preamble = ["<!-- Generated by scripts/dependency-inventory.py; do not edit rows by hand. -->",
                "# Cargo library inventory", "",
                f"{len(rows)} third-party package/version/source records; "
                f'{len({row["name"] for row in rows})} distinct crate names.', "",
                "Includes normal, build and development/test dependencies selected by Cargo tree",
                f"across {len(SCOPES)} workspaces. Existing workspaces retain Windows x64/Linux x86-64",
                "captures; only `native-macos` uses macOS x86-64/arm64 captures. This does not",
                "establish macOS application support or native runtime qualification. This is a development",
                "inventory, not a claim that every entry is linked into the release executable.",
                "Unused metadata feature-union/lockfile entries and project-owned crates are excluded.", "",
                "`app` = application workspace; `api` = public API/SDK and its tools;",
                "`renderer-tests` = vendored FemtoVG and its independent upstream/GPU tests;",
                "`basic`, `source`, `markdown` = independent viewer plugins;",
                "`conformance` = independent viewer ABI test fixture;",
                "`native-windows`, `native-linux`, `native-macos` = independent native filesystem plugins;",
                "`network` = independent Windows/Linux Network Neighborhood filesystem plugin;",
                "`filesystem-api`, `worker-api`, `thumbnail-api`, `image-viewer-api`, `web-viewer-api` = public protocols/SDK;",
                "`archive`, `thumbnail-image`, `thumbnail-video`, `image-viewer`, `html-viewer` = independent filesystem/media plugins.", "",
                "Licenses are exact upstream Cargo declarations, not automatic legal approval.",
                "`OR` gives alternatives; `AND` requires combined terms. Legacy slash syntax is",
                "retained verbatim. See [policy](../DEPENDENCY_POLICY.md) and",
                "[native/runtime dependencies and Slint choice](../THIRD_PARTY.md).", "",
                "## Workspace inputs", "",
                "These links identify the maintained manifests/locks. Captures must be refreshed",
                "after dependency or feature changes; `--check` compares tables with supplied",
                "captures and does not prove that old captures match today's source files.", "",
                "| Manifest | Lockfile | Captured targets |", "| --- | --- | --- |"]
    for scope, manifest in SCOPES.items():
        lock = Path(manifest).with_name("Cargo.lock").as_posix()
        targets = ", ".join(SCOPE_TARGETS.get(scope, TARGETS).values())
        preamble.append(f"| [{manifest}](../../{manifest}) | [{lock}](../../{lock}) | {targets} |")
    preamble += ["", "## Complete list", "",
                 "| Library | Version | Declared license | Workspaces | Targets | Direct |",
                 "| --- | --- | --- | --- | --- | --- |"]
    direct = ["<!-- Generated by scripts/dependency-inventory.py; do not edit rows by hand. -->",
              "# Direct third-party dependencies", "",
              "Direct means declared by at least one project-owned workspace member, including",
              "build/test tools. Transitive entries are in [the complete list](CARGO_LIBRARIES.md).",
              "Licenses retain upstream alternatives; see [policy](../DEPENDENCY_POLICY.md).", "",
              "| Library | Version | Declared license | Declared by |",
              "| --- | --- | --- | --- |"]
    fields = ["name", "version", "license", "source", "scopes", "targets", "direct_by"]
    csv_buffer = io.StringIO(newline="")
    writer = csv.DictWriter(csv_buffer, fieldnames=fields, lineterminator="\n")
    writer.writeheader()
    for row in rows:
        preamble.append("| " + " | ".join([
            library(row), cell(row["version"]), cell(row["license"]),
            ", ".join(row["scopes"]), ", ".join(row["targets"]),
            "yes" if row["direct_by"] else "—"]) + " |")
        if row["direct_by"]:
            direct.append("| " + " | ".join([library(row), cell(row["version"]),
                          cell(row["license"]), cell(", ".join(row["direct_by"]))]) + " |")
        csv_row = {key: "; ".join(value) if isinstance(value, list) else value
                   for key, value in row.items()}
        writer.writerow({key: "'" + value if value.startswith(("=", "+", "-", "@")) else value
                         for key, value in csv_row.items()})
    return {"CARGO_LIBRARIES.md": "\n".join(preamble) + "\n",
            "DIRECT_DEPENDENCIES.md": "\n".join(direct) + "\n",
            "cargo-licenses.csv": csv_buffer.getvalue()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, default=ROOT / "docs/dependencies")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    captures = {}
    selections = {}
    spec = importlib.util.spec_from_file_location("release_packages", ROOT / "scripts/package-release.py")
    release = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(release)
    for scope in SCOPES:
        for triple, target in SCOPE_TARGETS.get(scope, TARGETS).items():
            path = args.metadata_dir / f"{scope}-{triple}.json"
            captures[scope, target] = json.loads(path.read_text(encoding="utf-8-sig"))
            tree = path.with_suffix(".tree").read_text(encoding="utf-8-sig")
            selected = release.dependency_packages(captures[scope, target], tree,
                                                   required_root=None, include_local=True)
            selections[scope, target] = {identity(item) for item in selected}
    rows = inventory(captures, selections)
    unknown = [f'{row["name"]} {row["version"]}' for row in rows if row["license"] == "UNKNOWN"]
    if unknown:
        raise ValueError("Missing license metadata requires source review: " + ", ".join(unknown))
    if not args.check:
        args.output_dir.mkdir(parents=True, exist_ok=True)
    for name, text in render(rows).items():
        path = args.output_dir / name
        if args.check:
            if not path.is_file() or path.read_text(encoding="utf-8") != text:
                raise ValueError(f"Inventory is stale: {path}")
        else:
            path.write_text(text, encoding="utf-8", newline="\n")
    print(json.dumps({"packages": len(rows), "names": len({row["name"] for row in rows}),
                      "direct": sum(bool(row["direct_by"]) for row in rows),
                      "licenses": sorted({row["license"] for row in rows})}, indent=2))


if __name__ == "__main__":
    main()
