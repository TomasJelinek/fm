#!/usr/bin/env python3
"""Run the verified private Meson with isolated Python startup and regeneration."""
import sys

if not sys.flags.isolated or not sys.flags.no_site or not sys.dont_write_bytecode:
    raise SystemExit('Private Meson requires Python -I -S -B')

import importlib.util
import os
from pathlib import Path


def main():
    script = Path(__file__).resolve()
    spec = importlib.util.spec_from_file_location('native_bootstrap', script.with_name('bootstrap-tools.py'))
    bootstrap = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = bootstrap
    spec.loader.exec_module(bootstrap)
    value = os.environ.get('MIDNIGHT_IMAGE_NATIVE_TOOLS')
    if not value:
        raise ValueError('Run the private tool bootstrap before building dav1d')
    root = bootstrap.ordinary_root(value)
    artifact = bootstrap.MESON
    data = bootstrap.checked_bytes(root / 'downloads' / artifact.filename, artifact)
    entries = bootstrap.archive_entries(artifact, data)
    package = root / 'tools' / artifact.name
    if package.resolve() != package or not package.is_dir():
        raise ValueError('Private Meson directory is missing or linked')
    expected = {name: payload for name, is_dir, payload, _, _ in entries if not is_dir}
    bootstrap.check_tree(package, expected)
    sys.path.insert(0, str(package))
    from mesonbuild import mesonmain
    from mesonbuild.utils import universal
    # Meson writes this command into Ninja for regeneration and internal helpers.
    universal.python_command[:] = [sys.executable, '-I', '-S', '-B']
    return mesonmain.run(sys.argv[1:], str(script))


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print('Private Meson: ' + str(error), file=sys.stderr)
        sys.exit(1)
