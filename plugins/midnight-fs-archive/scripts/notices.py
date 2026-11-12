"""Preserve exact pinned dependency license/notice files in a staged plugin package."""
import argparse
import json
import pathlib
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--config", required=True)
parser.add_argument("--output", required=True)
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parents[1]
output = pathlib.Path(args.output).resolve()
if "_build" not in output.parts:
    raise SystemExit("Distribution notices must be staged under _build")
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--format-version", "1", "--locked", "--offline",
    "--manifest-path", str(root / "Cargo.toml"), "--config", args.config,
], text=True))
inventory = []
for package in metadata["packages"]:
    if package["source"] is None or "midnight-explorer.invalid" in package["source"]:
        continue
    license_name = package.get("license")
    if not license_name:
        raise SystemExit(f"Missing license metadata: {package['name']}")
    source = pathlib.Path(package["manifest_path"]).parent
    files = sorted(p for p in source.iterdir() if p.is_file()
                   and p.name.lower().startswith(("license", "licence", "copying", "notice", "copyright")))
    if package.get("license_file"):
        files.append(source / package["license_file"])
    if not files:
        raise SystemExit(f"Pinned license files must be reviewed: {package['name']}")
    target = output / f"{package['name']}-{package['version']}"
    target.mkdir(parents=True, exist_ok=True)
    for path in files:
        shutil.copyfile(path, target / path.name)
    inventory.append({"name": package["name"], "version": package["version"], "license": license_name})
output.mkdir(parents=True, exist_ok=True)
(output / "inventory.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
