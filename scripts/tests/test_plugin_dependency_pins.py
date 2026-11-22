"""Independent plugins pin every registry dependency exactly, and their lockfiles agree.

The dependency policy asks for exact pins (docs/DEPENDENCY_POLICY.md): a plugin's own
lockfile is what its CI and packages build, so a looser requirement in a manifest would let
a later lock refresh pick another version, with other licenses or notices, unnoticed.
"""
from pathlib import Path
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]
KINDS = ("dependencies", "dev-dependencies", "build-dependencies")


def declarations(manifest):
    """Every (table, name, specification) of a manifest, platform-specific ones included."""
    for kind in KINDS:
        for name, specification in manifest.get(kind, {}).items():
            yield kind, name, specification
    for target, section in manifest.get("target", {}).items():
        for kind in KINDS:
            for name, specification in section.get(kind, {}).items():
                yield f"target.{target}.{kind}", name, specification


def registry_requirements(manifest):
    """(table, package, version requirement) of each dependency taken from a registry."""
    for table, name, specification in declarations(manifest):
        if isinstance(specification, str):
            yield table, name, specification
        elif "path" not in specification and "git" not in specification:
            yield table, specification.get("package", name), specification.get("version")


class PluginDependencyPins(unittest.TestCase):
    def test_every_plugin_pins_its_registry_dependencies_to_the_locked_version(self):
        manifests = sorted(ROOT.glob("plugins/*/Cargo.toml"))
        self.assertIn(ROOT / "plugins/midnight-thumbnail-video/Cargo.toml", manifests)
        for path in manifests:
            manifest = tomllib.loads(path.read_text(encoding="utf-8"))
            lock = tomllib.loads(path.with_name("Cargo.lock").read_text(encoding="utf-8"))
            # Build metadata (cef's "+156.0.26") takes no part in matching a requirement.
            locked = {(package["name"], package["version"].split("+")[0])
                      for package in lock.get("package", [])}
            for table, package, requirement in registry_requirements(manifest):
                with self.subTest(plugin=path.parent.name, table=table, package=package):
                    self.assertIsInstance(requirement, str)
                    self.assertTrue(requirement.startswith("="), requirement)
                    self.assertIn((package, requirement[1:].strip()), locked)


if __name__ == "__main__":
    unittest.main()
