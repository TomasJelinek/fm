#!/usr/bin/env python3
"""Run Cargo's workspace test targets with bounded successful integration artifacts.

CI explicitly opts into retiring reported test executables/PDBs; sources, libraries,
failed artifacts and unrelated output files are never cleanup candidates. Local full
validation (scripts/validate.*) runs the same gates with --keep-test-artifacts, which
retires nothing, and --no-fail-fast, which runs every gate after a failure.
"""
import argparse
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import time
import tomllib

LIB_KINDS = {'lib', 'rlib', 'dylib', 'cdylib', 'staticlib', 'proc-macro'}
WINDOWS_SHARING_ERRORS = (32, 33) if os.name == 'nt' else ()
DEFAULT_BATCH_SIZE = 8


def plan(metadata):
    members = set(metadata['workspace_members'])
    packages = [package for package in metadata['packages'] if package['id'] in members]
    features = metadata['ci_active_features']
    def eligible(package, target, field='test'):
        return target.get(field, False) and set(target.get('required-features', ())) <= features[package['id']]
    def gate(args, name='', selected=(), cleanup=False):
        result = {'args': args, 'name': name, 'packages': set(selected), 'cleanup': cleanup}
        if cleanup:
            result['targets'] = {name: set(selected)}
        return result
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


def batch_integrations(gates, batch_size):
    """Bound live executables while retaining Cargo's workspace target selection.

    A same-name group is indivisible, just as in serial execution. Its existing
    size may exceed the requested batch size; no additional group joins it.
    """
    if type(batch_size) is not int or not 1 <= batch_size <= 8:
        raise ValueError('Integration batch size must be an integer from 1 through 8')
    result, pending = [], []
    count = 0
    def flush():
        if not pending:
            return
        if len(pending) == 1:
            result.append(pending[0])
        else:
            targets = {name: packages for gate in pending
                       for name, packages in gate['targets'].items()}
            result.append({
                'args': [part for name in targets for part in ('--test', name)]
                        + ['--message-format=json'],
                'name': ', '.join(targets), 'targets': targets,
                'packages': set().union(*targets.values()), 'cleanup': True})
        pending.clear()
    for gate in gates:
        if not gate['cleanup']:
            flush()
            count = 0
            result.append(gate)
            continue
        size = len(integration_identities(gate))
        if pending and count + size > batch_size:
            flush()
            count = 0
        pending.append(gate)
        count += size
    flush()
    return result


def integration_identities(gate):
    return {(package, name) for name, packages in gate['targets'].items()
            for package in packages}


def is_link(metadata):
    """A symbolic link or any Windows reparse point, such as a junction."""
    return stat.S_ISLNK(metadata.st_mode) or bool(
        getattr(metadata, 'st_file_attributes', 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def in_checkout(value, root):
    """`value` spelled below the resolved checkout `root`, or None if it escapes it.

    Cargo reports paths as it was invoked (CARGO_TARGET_DIR may name the checkout through
    Fedora Silverblue's /home -> /var/home, a linked parent folder or a subst drive), so
    the checkout is recognised however a path spells it. Below the checkout no component
    may be a link, a junction or another reparse point, or '..'.
    """
    path = Path(value)
    if not path.is_absolute() or '..' in path.parts:
        return None
    checkout = os.stat(root)
    ancestors = (path, *path.parents)
    # The shallowest match: a link below the checkout back to it is inspected, not trusted.
    for depth in reversed(range(len(ancestors))):
        try:
            if os.path.samestat(ancestors[depth].stat(), checkout):
                break
        except OSError:
            continue
    else:
        return None
    result = Path(root)
    for part in path.relative_to(ancestors[depth]).parts:
        result = result / part
        try:
            if is_link(result.lstat()):
                return None
        except FileNotFoundError:
            continue
    return result


def plain_directories(*paths):
    """Whether every path is an existing directory that is not a link."""
    for path in paths:
        try:
            metadata = path.lstat()
        except FileNotFoundError:
            return False
        if is_link(metadata) or not stat.S_ISDIR(metadata.st_mode):
            return False
    return True


def plain_file(path):
    """The (device, inode) identity of a regular file that is not a link, or None."""
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return None
    if is_link(metadata) or not stat.S_ISREG(metadata.st_mode):
        return None
    return metadata.st_dev, metadata.st_ino


def output_root(root, metadata):
    """The checkout's own _build, which Cargo's target directory must be, however spelled."""
    root = root.resolve(strict=True)
    expected = root / '_build'
    if in_checkout(metadata['target_directory'], root) != expected or not plain_directories(expected):
        raise ValueError('CI artifact retirement requires the plain repository _build target directory')
    return expected


def retire(root, gate, artifacts):
    """Delete the reported executables of a passing gate; `root` is the checkout's _build."""
    deps = root / 'debug/deps'
    if not plain_directories(root, deps.parent, deps):
        raise ValueError('Generated test directory must not redirect outside _build/debug/deps')
    expected = integration_identities(gate)
    selected = set()
    paths = set()
    for artifact in artifacts:
        identity = (artifact.get('package_id'), artifact.get('target', {}).get('name'))
        if (artifact.get('reason') != 'compiler-artifact'
                or artifact.get('target', {}).get('kind') != ['test']
                or not artifact.get('profile', {}).get('test')
                or identity not in expected or identity in selected):
            raise ValueError('Unexpected integration artifact identity')
        value = artifact.get('executable')
        if not isinstance(value, str):
            raise ValueError('Missing integration executable artifact')
        # Spelled below the resolved checkout, so the comparison with deps is exact.
        executable = in_checkout(value, root.parent)
        pattern = re.escape(identity[1].replace('-', '_')) + r'-[0-9a-f]+(?:\.exe)?'
        if (executable is None or executable.parent != deps or plain_file(executable) is None
                or not re.fullmatch(pattern, executable.name)):
            raise ValueError(f'Unowned integration artifact: {value}')
        if executable in paths:
            raise ValueError('Duplicate integration executable artifact')
        paths.add(executable)
        if executable.suffix == '.exe':
            pdb = executable.with_suffix('.pdb')
            if os.path.lexists(pdb):
                if plain_file(pdb) is None:
                    raise ValueError(f'Unowned integration symbols: {pdb}')
                paths.add(pdb)
        selected.add(identity)
    if selected != expected:
        raise ValueError('Successful Cargo gate did not report every selected integration artifact')
    # Validate the complete set before deleting anything. No globs or recursive removal.
    identities = {path: artifact_identity(path) for path in paths}
    for path in sorted(paths):
        unlink_artifact(path, deps, identities[path])
    print(f'Retired {len(paths)} generated files for {gate["name"]}', flush=True)


def artifact_identity(path):
    identity = plain_file(path)
    if identity is None:
        raise ValueError(f'Unowned or changed generated artifact: {path}')
    return identity


def unlink_artifact(path, deps, identity):
    # A completed executable can still have a short native sharing lock. Retrying
    # this one validated object keeps the per-gate disk bound without hiding errors.
    first_error = None
    deadline = 0.0
    while True:
        if first_error is not None and time.monotonic() >= deadline:
            raise first_error
        if (not plain_directories(deps.parent.parent, deps.parent, deps)
                or path.parent != deps or plain_file(path) != identity):
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
                if len(artifacts) < len(integration_identities(gate)):
                    artifacts.append(event)
                else:
                    overflow = True
            elif event.get('reason') not in ('compiler-artifact', 'build-script-executed', 'build-finished'):
                print(line, end='', flush=True)  # Preserve JSON diagnostics emitted by tests.
        code = process.wait()
    if code == 0 and overflow:
        raise ValueError('Cargo reported more integration artifacts than selected targets')
    return code, artifacts


def feature_arguments(metadata):
    members = set(metadata['workspace_members'])
    names = {package['id']: package['name'] for package in metadata['packages'] if package['id'] in members}
    # Excluding a disabled unit/doctest target must not drop its member's default
    # features from other members that depend on it with default-features=false.
    features = sorted(f'{names[identity]}/{feature}'
                      for identity, values in metadata['ci_active_features'].items()
                      for feature in values)
    return ['--features', ','.join(features)] if features else []


def artifact_selection(metadata, package_name, target):
    """Select one test executable with exactly the workspace gates' Cargo selection.

    A `-p PACKAGE` build resolves features for that package's graph only, so shared
    dependencies get other feature sets and everything above them is rebuilt.
    """
    members = set(metadata['workspace_members'])
    packages = [package for package in metadata['packages']
                if package['id'] in members and package['name'] == package_name]
    if len(packages) != 1:
        raise ValueError(f'Unknown workspace package: {package_name}')
    package = packages[0]
    kind, _, name = target.partition(':')
    if kind == 'lib' and not name:
        gates = [gate for gate in plan(metadata) if gate['args'][:1] == ['--lib']]
        if not gates or package_name in gates[0]['args'][2::2]:
            raise ValueError(f'{package_name} has no enabled library test target')
        return package, gates[0]['args'], lambda artifact: bool(LIB_KINDS.intersection(artifact['target']['kind']))
    if kind in ('test', 'bin') and name:
        if not any(kind in candidate['kind'] and candidate['name'] == name for candidate in package['targets']):
            raise ValueError(f'{package_name} has no {kind} target {name}')
        return (package, ['--' + kind, name],
                lambda artifact: artifact['target']['kind'] == [kind] and artifact['target']['name'] == name)
    raise ValueError('Expected a target of lib, test:NAME or bin:NAME')


def build_artifact(root, metadata, package_name, target, invoke=subprocess.run):
    """Build (normally: reuse) and return one test executable and its package directory."""
    package, args, matches = artifact_selection(metadata, package_name, target)
    command = (['cargo', 'test', '--workspace', '--locked'] + feature_arguments(metadata) + args
               + ['--no-run', '--message-format=json'])
    print('+ ' + ' '.join(command), file=sys.stderr, flush=True)
    result = invoke(command, cwd=root, stdout=subprocess.PIPE, text=True, encoding='utf-8')
    if result.returncode:
        raise ValueError(f'Cargo could not build {package_name} {target} (exit {result.returncode})')
    executables = set()
    for line in result.stdout.splitlines():
        try:
            event = json.loads(line) if line.startswith('{') else None
        except json.JSONDecodeError:
            event = None
        if (isinstance(event, dict) and event.get('reason') == 'compiler-artifact'
                and event.get('package_id') == package['id'] and event.get('profile', {}).get('test')
                and isinstance(event.get('executable'), str) and matches(event)):
            executables.add(event['executable'])
    if len(executables) != 1:
        raise ValueError(f'Cargo reported {len(executables)} executables for {package_name} {target}')
    return Path(executables.pop()), Path(package['manifest_path']).parent


def execute(root, metadata, invoke=run_command, batch_size=DEFAULT_BATCH_SIZE,
            retire_artifacts=True, fail_fast=True):
    """Run every gate; return the first failing Cargo exit status, or 0.

    `retire_artifacts=False` keeps every executable (no target-directory requirement).
    `fail_fast=False` adds Cargo's --no-fail-fast and continues with later gates.
    """
    target_root = output_root(root, metadata) if retire_artifacts else None
    serial = plan(metadata)
    gates = batch_integrations(serial, batch_size)
    feature_args = feature_arguments(metadata)
    print(f'Workspace gates: {len(gates)}; '
          f'integration names: {sum(gate["cleanup"] for gate in serial)}; '
          f'integration batch bound: {batch_size}', flush=True)
    first_failure = 0
    for index, gate in enumerate(gates, 1):
        command = ['cargo', 'test', '--workspace', '--locked'] + feature_args + gate['args']
        if not fail_fast:
            command.append('--no-fail-fast')
        started = time.monotonic()
        code, artifacts = invoke(command, root, gate)
        finished = time.monotonic()
        print(f'Gate {index}/{len(gates)} Cargo exit {code}; '
              f'elapsed {finished - started:.3f}s', flush=True)
        if code:
            if fail_fast:
                return code  # Retain failed output for diagnosis; never skip to later targets.
            first_failure = first_failure or code
            continue  # A failed gate's artifacts are never retired.
        if gate['cleanup'] and retire_artifacts:
            retire(target_root, gate, artifacts)
            print(f'Gate {index}/{len(gates)} retirement '
                  f'{time.monotonic() - finished:.3f}s', flush=True)
    return first_failure


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


def artifact_main(arguments, invoke=subprocess.run, metadata_loader=None):
    """`artifact PACKAGE TARGET` prints a test executable built with the gates' selection;
    `run PACKAGE TARGET [LIBTEST ARGS...]` runs it from its package directory, as Cargo would.

    Post-gate steps use this instead of `cargo test -p PACKAGE`, which would rebuild
    shared dependencies with that package's narrower feature set."""
    mode, rest = arguments[0], arguments[1:]
    if len(rest) < 2 or (mode == 'artifact' and len(rest) != 2):
        print(f'Usage: ci-workspace-tests.py {mode} PACKAGE lib|test:NAME|bin:NAME'
              + (' [LIBTEST ARGS...]' if mode == 'run' else ''), file=sys.stderr)
        return 2
    root = Path(__file__).resolve().parents[1]
    metadata = (metadata_loader or load_metadata)(root)
    executable, directory = build_artifact(root, metadata, rest[0], rest[1], invoke)
    if mode == 'artifact':
        print(executable, flush=True)
        return 0
    return invoke([str(executable), *rest[2:]], cwd=directory).returncode


def positive_count(value):
    """A whole number of at least one, as --jobs takes it."""
    if not value.isdigit() or int(value) < 1:
        raise argparse.ArgumentTypeError('needs a positive whole number')
    return int(value)


def main():
    configure_output()
    if sys.argv[1:2] in (['artifact'], ['run']):
        return artifact_main(sys.argv[1:])
    parser = argparse.ArgumentParser(description=__doc__)
    retention = parser.add_mutually_exclusive_group()
    retention.add_argument('--retire-successful-test-artifacts', action='store_true',
                           help='CI only (CI=true): delete each passing integration executable')
    retention.add_argument('--keep-test-artifacts', action='store_true',
                           help='Local validation: the same gates, retiring nothing')
    parser.add_argument('--no-fail-fast', action='store_true',
                        help='Run every gate and test target after a failure; '
                             'exit with the first failing status')
    parser.add_argument('--integration-batch-size', type=int, choices=(1, 4, 8),
                        default=DEFAULT_BATCH_SIZE,
                        help='Maximum selected integrations per Cargo command, except atomic '
                             'same-name groups (default: %(default)s)')
    parser.add_argument('--jobs', type=positive_count, metavar='N',
                        help='Build with N Cargo jobs and run each test executable with N test '
                             'threads (default: every logical CPU, or CARGO_BUILD_JOBS and '
                             'RUST_TEST_THREADS when set)')
    args = parser.parse_args()
    if args.jobs:
        os.environ['CARGO_BUILD_JOBS'] = os.environ['RUST_TEST_THREADS'] = str(args.jobs)
    if not args.keep_test_artifacts and (not args.retire_successful_test_artifacts
                                         or os.environ.get('CI', '').lower() != 'true'):
        parser.error('Requires CI=true and explicit --retire-successful-test-artifacts, '
                     'or --keep-test-artifacts')
    root = Path(__file__).resolve().parents[1]
    return execute(root, load_metadata(root), batch_size=args.integration_batch_size,
                   retire_artifacts=args.retire_successful_test_artifacts,
                   fail_fast=not args.no_fail_fast)

if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'CI workspace tests: {error}', file=sys.stderr)
        sys.exit(1)