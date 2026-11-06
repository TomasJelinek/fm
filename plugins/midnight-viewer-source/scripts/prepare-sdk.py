#!/usr/bin/env python3
"""Prepare dependency-source config using only a released public SDK registry."""
import pathlib
import sys
import tomllib
import json

config = pathlib.Path(sys.argv[1]).resolve()
build = pathlib.Path(sys.argv[2]).resolve()
data = tomllib.loads(config.read_text())
registry = pathlib.Path(data["source"]["midnight-releases"]["local-registry"])
if not registry.is_absolute():
    registry = config.parent.parent / registry
registry = registry.resolve(strict=True)
build.mkdir(parents=True, exist_ok=True)
target = build / "sdk.config.toml"
target.write_text(
    '[registries.midnight]\nindex = "https://midnight-explorer.invalid/api-index"\n'
    '[source.midnight]\nregistry = "https://midnight-explorer.invalid/api-index"\n'
    'replace-with = "midnight-releases"\n[source.midnight-releases]\nlocal-registry = '
    + json.dumps(str(registry)) + "\n"
)
print(target)
