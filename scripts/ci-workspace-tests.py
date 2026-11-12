#!/usr/bin/env python3
"""Run Cargo's workspace test targets with bounded successful integration artifacts.

CI explicitly opts into retiring reported test executables/PDBs; sources, libraries,
failed artifacts and unrelated output files are never cleanup candidates.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import tomllib

LIB_KINDS = {'lib', 'rlib', 'dylib', 'cdylib', 'staticlib', 'proc-macro'}
WINDOWS_SHARING_ERRORS = (32, 33) if os.name == 'nt' else ()


def plan(metadata):
    members = set(metadata['workspace_members'])
    packages = [package for package in metadata['packages'] if package['id'] in members]
    features = metadata['ci_active_features']
    def eligible(package, target, field='test'):
        return target.get(field, False) and set(target.get('required-features', ())) <= features[package['id']]
    def gate(args, name='', selected=(), cleanup=False):
        return {'args': args, 'name': name, 'packages': set(selected), 'cleanup': cleanup}
    result = []
    libraries = [(package, target) for package in packages for target in package['targets']
                 if LIB_KINDS.intersection(target['kind'])]
    for flag, field in [('--lib', 'test'), ('--doc', 'doctest')]:
        if any(eligible(package, target, field) for package, target in libraries):
            excluded = sorted({package['name'] for package, target in libraries
                               if not eligible(package, target, field)})
            result.append(gate([flag] + [part for name in excluded for part in ('--exclude', name)]))
    if any('example' in target['kind'] for package in packages for target in package['targets']):
        # Cargo normally compiles examples without executing their main functions.
        result.append(gate(['--examples', '--no-run']))
    for kind, flag in [('bin', '--bin'), ('example', '--example'), ('test', '--test')]:
        groups = {}
        for package in packages:
            for target in package['targets']:
                if kind in target['kind']:
                    groups.setdefault(target['name'], []).append((package, target))
        for name, targets in sorted(groups.items()):
            selected = {package['id'] for package, target in targets if eligible(package, target)}
            if not selected:
                continue
            if len(selected) != len(targets):
                raise ValueError(f'Mixed enabled/disabled {kind} target {name}; Cargo explicit selection would change default coverage')
            args = [flag, name] + (['--message-format=json'] if kind == 'test' else [])
            result.append(gate(args, name, selected, kind == 'test'))
    return result


def output_root(root, metadata):
    root = root.resolve(strict=True)
    expected = root / '_build'
    actual = Path(metadata['target_directory'])
    if expected.is_symlink() or not actual.is_absolute() or actual.resolve() != expected:
        raise ValueError('CI artifact retirement requires the plain repository _build target directory')
    return expected


def retire(root, gate, artifacts):
    deps = root / 'debug/deps'
    if deps.is_symlink() or deps.resolve() != deps:
        raise ValueError('Generated test directory must not redirect outside _build/debug/deps')
    selected = set()
    paths = set()
    for artifact in artifacts:
        if (artifact.get('reason') != 'compiler-artifact'
                or artifact.get('target', {}).get('kind') != ['test']
                or artifact['target'].get('name') != gate['name']
                or not artifact.get('profile', {}).get('test')
                or artifact.get('package_id') not in gate['packages']):
            raise ValueError('Unexpected integration artifact identity')
        value = artifact.get('executable')
        if not isinstance(value, str):
            raise ValueError('Missing integration executable artifact')
        executable = Path(value)
        pattern = re.escape(gate['name'].replace('-', '_')) + r'-[0-9a-f]+(?:\.exe)?'
        if (not executable.is_absolute() or executable.is_symlink()
                or executable.parent != deps or executable.resolve(strict=True) != executable
                or not executable.is_file() or not re.fullmatch(pattern, executable.name)):
            raise ValueError(f'Unowned integration artifact: {executable}')
        paths.add(executable)
        if executable.suffix == '.exe':
            pdb = executable.with_suffix('.pdb')
            if pdb.is_symlink():
                raise ValueError(f'Unowned integration symbols: {pdb}')
            if pdb.exists():
                if not pdb.is_file() or pdb.resolve(strict=True) != pdb:
                    raise ValueError(f'Unowned integration symbols: {pdb}')
                paths.add(pdb)
        selected.add(artifact['package_id'])
    if selected != gate['packages']:
        raise ValueError('Successful Cargo gate did not report every selected integration artifact')
    # Validate the complete set before deleting anything. No globs or recursive removal.
    identities = {path: artifact_identity(path) for path in paths}
    for path in sorted(paths):
        unlink_artifact(path, deps, identities[path])
    print(f'Retired {len(paths)} generated files for {gate["name"]}', flush=True)


def artifact_identity(path):
    metadata = path.lstat()
    return metadata.st_dev, metadata.st_ino


def unlink_artifact(path, deps, identity):
    # A completed executable can still have a short native sharing lock. Retrying
    # this one validated object keeps the per-gate disk bound without hiding errors.
    first_error = None
    deadline = 0.0
    while True:
        if first_error is not None and time.monotonic() >= deadline:
            raise first_error
        if (deps.is_symlink() or deps.resolve(strict=True) != deps
                or path.parent != deps or path.is_symlink() or not path.is_file()
                or path.resolve(strict=True) != path or artifact_identity(path) != identity):
            raise ValueError(f'Unowned or changed generated artifact: {path}')
        try:
            path.unlink()
            return
        except OSError as error:
            if getattr(error, 'winerror', None) not in WINDOWS_SHARING_ERRORS:
                raise
            now = time.monotonic()
            if first_error is None:
                first_error = error
                deadline = now + 2.0
            remaining = deadline - now
            if remaining <= 0:
                raise first_error
            time.sleep(min(0.1, remaining))


def run_command(command, root, gate):
    artifacts = []
    overflow = False
    print('+ ' + ' '.join(command), flush=True)
    with subprocess.Popen(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, encoding='utf-8', errors='replace') as process:
        for line in process.stdout:
            try:
                event = json.loads(line) if line.startswith('{') else None
            except json.JSONDecodeError:
                event = None
            if not isinstance(event, dict):
                print(line, end='', flush=True)
                continue
            if event.get('reason') == 'compiler-message':
                print(event.get('message', {}).get('rendered', line), end='', flush=True)
            elif gate['cleanup'] and event.get('reason') == 'compiler-artifact' and event.get('target', {}).get('kind') == ['test']:
                if len(artifacts) < len(gate['packages']):
                    artifacts.append(event)
                else:
                    overflow = True
            elif event.get('reason') not in ('compiler-artifact', 'build-script-executed', 'build-finished'):
                print(line, end='', flush=True)  # Preserve JSON diagnostics emitted by tests.
        code = process.wait()
    if code == 0 and overflow:
        raise ValueError('Cargo reported more integration artifacts than selected targets')
    return code, artifacts


def execute(root, metadata, invoke=run_command):
    target_root = output_root(root, metadata)
    gates = plan(metadata)
    members = set(metadata['workspace_members'])
    names = {package['id']: package['name'] for package in metadata['packages'] if package['id'] in members}
    # Excluding a disabled unit/doctest target must not drop its member's default
    # features from other members that depend on it with default-features=false.
    features = sorted(f'{names[identity]}/{feature}'
                      for identity, values in metadata['ci_active_features'].items()
                      for feature in values)
    feature_args = ['--features', ','.join(features)] if features else []
    print(f'Workspace gates: {len(gates)}; integration names: {sum(gate["cleanup"] for gate in gates)}', flush=True)
    for gate in gates:
        command = ['cargo', 'test', '--workspace', '--locked'] + feature_args + gate['args']
        code, artifacts = invoke(command, root, gate)
        if code:
            return code  # Retain failed output for diagnosis; never skip to later targets.
        if gate['cleanup']:
            retire(target_root, gate, artifacts)
    return 0


def apply_feature_tree(metadata, tree):
    """Cargo metadata includes inactive platform features even with --filter-platform.

    Depth-zero cargo tree roots describe active workspace member features, without
    merging transitive build/proc-macro variants into those root selections.
    """
    members = set(metadata['workspace_members'])
    packages = {package['name']: package for package in metadata['packages'] if package['id'] in members}
    active = {}
    for line in tree.splitlines():
        if not line.strip():
            continue
        description, separator, values = line.rpartition('|')
        name = description.split(' ', 1)[0]
        package = packages.get(name)
        if not separator or package is None or not description.startswith(f'{name} v{package["version"]} '):
            raise ValueError(f'Unexpected workspace feature-tree root: {line}')
        expected = '(' + str(Path(package['manifest_path']).parent).replace('\\', '/') + ')'
        observed = description.replace('\\', '/')
        if os.name == 'nt':
            expected, observed = expected.casefold(), observed.casefold()
        features = set(filter(None, values.removesuffix(' (*)').split(',')))
        if expected not in observed or not features <= package['features'].keys() or package['id'] in active:
            raise ValueError(f'Ambiguous workspace feature-tree root: {line}')
        active[package['id']] = features
    if set(active) != members:
        raise ValueError('Cargo feature tree omitted a workspace member')
    metadata['ci_active_features'] = active


def require_native_configuration(root, environment=None):
    environment = os.environ if environment is None else environment
    if environment.get('CARGO_BUILD_TARGET'):
        raise ValueError('This native CI test runner requires CARGO_BUILD_TARGET to be unset')
    root = root.resolve(strict=True)
    cargo_home = Path(environment.get('CARGO_HOME', Path.home() / '.cargo'))
    if not cargo_home.is_absolute():
        cargo_home = root / cargo_home
    directories = [parent / '.cargo' for parent in (root, *root.parents)] + [cargo_home]
    for directory in dict.fromkeys(directories):
        # Cargo prefers the extensionless file when both spellings exist.
        config = directory / 'config'
        if not config.is_file():
            config = directory / 'config.toml'
        if not config.is_file():
            continue
        with config.open('rb') as stream:
            values = tomllib.load(stream)
        if 'include' in values:
            raise ValueError(f'Native CI artifact retirement does not support Cargo config include: {config}')
        if 'target' in values.get('build', {}):
            raise ValueError(f'Native CI artifact retirement requires build.target to be unset: {config}')


def load_metadata(root):
    require_native_configuration(root)
    version = subprocess.check_output(['rustc', '-vV'], cwd=root, text=True, encoding='utf-8')
    host = next((line.removeprefix('host: ') for line in version.splitlines() if line.startswith('host: ')), None)
    if not host:
        raise ValueError('rustc did not report the native host target')
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version=1', '--locked', '--filter-platform', host],
        cwd=root, text=True, encoding='utf-8'))
    tree = subprocess.check_output(
        ['cargo', 'tree', '--workspace', '--locked', '--target', host, '--edges', 'normal,build,dev',
         '--prefix', 'none', '--depth', '0', '--format', '{p}|{f}'], cwd=root, text=True, encoding='utf-8')
    apply_feature_tree(metadata, tree)
    return metadata

def configure_output():
    # Cargo emits UTF-8. Hosted Windows runners pipe Python output through a
    # legacy code page by default; a Unicode test name must not abort the suite.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, 'reconfigure'):
            stream.reconfigure(encoding='utf-8', errors='backslashreplace')


def main():
    configure_output()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--retire-successful-test-artifacts', action='store_true')
    args = parser.parse_args()
    if not args.retire_successful_test_artifacts or os.environ.get('CI', '').lower() != 'true':
        parser.error('Requires CI=true and explicit --retire-successful-test-artifacts')
    root = Path(__file__).resolve().parents[1]
    return execute(root, load_metadata(root))

if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'CI workspace tests: {error}', file=sys.stderr)
        sys.exit(1)