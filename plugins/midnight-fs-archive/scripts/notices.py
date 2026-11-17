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
if "_build" not in output.parts[:-1]:
    raise SystemExit("Distribution notices must be staged under _build")
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--format-version", "1", "--locked", "--offline",
    "--manifest-path", str(root / "Cargo.toml"), "--config", args.config,
], text=True))
plan = {}
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
    plan[f"{package['name']}-{package['version']}"] = files
    inventory.append({"name": package["name"], "version": package["version"], "license": license_name})
# Every input is validated before the previous output is replaced. Only an earlier
# output of this script (identified by its inventory) is removed, so notices of a
# dependency that is no longer shipped never survive a rebuild.
if output.is_symlink() or (output.exists() and not output.is_dir()):
    raise SystemExit("Distribution notices output must be a directory")
if output.exists() and any(output.iterdir()):
    if not (output / "inventory.json").is_file():
        raise SystemExit(f"Refusing to replace a directory that is not generated notices: {output}")
    shutil.rmtree(output)
output.mkdir(parents=True, exist_ok=True)
for name, files in plan.items():
    target = output / name
    target.mkdir(parents=True, exist_ok=True)
    for path in files:
        shutil.copyfile(path, target / path.name)
(output / "inventory.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
