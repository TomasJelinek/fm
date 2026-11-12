#!/usr/bin/env python3
"""Run explicitly selected, already-built libtest artifacts with bounded parallelism."""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import threading
import time
import uuid

LIB_KINDS = {'lib', 'rlib', 'dylib', 'cdylib', 'staticlib', 'proc-macro'}
COUNTS = re.compile(
    r'^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; '
    r'(\d+) measured; (\d+) filtered out;(?:.*)$', re.MULTILINE)


@dataclass(frozen=True)
class Selection:
    package: str
    kind: str
    target: str
    filter: str | None = None


def available_cpus():
    if hasattr(os, 'process_cpu_count'):
        return max(1, os.process_cpu_count() or 1)
    if hasattr(os, 'sched_getaffinity'):
        return max(1, len(os.sched_getaffinity(0)))
    return max(1, os.cpu_count() or 1)


def target_kind(target):
    kinds = set(target.get('kind', ()))
    if kinds & LIB_KINDS:
        return 'lib'
    return next(iter(kinds)) if len(kinds) == 1 else None


def beneath(path, parent):
    return path == parent or parent in path.parents


def signature(path):
    value = path.stat()
    return value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read_receipts(paths):
    events = []
    for path in paths:
        with Path(path).open(encoding='utf-8-sig') as stream:
            for number, line in enumerate(stream, 1):
                if not line.lstrip().startswith('{'):
                    continue  # Cargo progress can share a redirected stderr stream.
                try:
                    event = json.loads(line)
                except json.JSONDecodeError as error:
                    raise ValueError(f'Malformed Cargo receipt {path}:{number}') from error
                if not isinstance(event, dict):
                    raise ValueError(f'Invalid Cargo receipt {path}:{number}')
                events.append(event)
    return events


def prepare(root, metadata, receipts, selections):
    root = root.resolve(strict=True)
    if Path(metadata['workspace_root']).resolve(strict=True) != root:
        raise ValueError('Metadata belongs to a different workspace')
    build = root / '_build'
    target = Path(metadata['target_directory'])
    if (not target.is_absolute() or target.resolve(strict=True) != target
            or not beneath(target, build) or build.resolve(strict=True) != build):
        raise ValueError('Test artifacts must reside in the ordinary workspace _build directory')
    finished = [event.get('success') for event in receipts if event.get('reason') == 'build-finished']
    if not finished or not all(value is True for value in finished):
        raise ValueError('Receipts must contain only successful completed Cargo builds')
    members = set(metadata['workspace_members'])
    packages = {}
    for package in metadata['packages']:
        if package['id'] in members:
            if package['name'] in packages:
                raise ValueError('Ambiguous workspace package name')
            packages[package['name']] = package
    if not selections:
        raise ValueError('At least one explicit package/target selector is required')
    groups = {}
    seen = set()
    for selected in selections:
        if selected in seen:
            raise ValueError(f'Repeated selector: {selected}')
        seen.add(selected)
        if selected.filter is not None and (not selected.filter or selected.filter.startswith('-')
                or len(selected.filter) > 4096 or any(ord(char) < 32 for char in selected.filter)):
            raise ValueError('A test filter must be nonempty text, not a libtest option')
        package = packages.get(selected.package)
        if package is None:
            raise ValueError(f'Unknown workspace package: {selected.package}')
        matches = [item for item in package['targets'] if item['name'] == selected.target
                   and target_kind(item) == selected.kind and item.get('test', False)]
        if len(matches) != 1:
            raise ValueError(f'Unknown or ambiguous test target: {selected}')
        manifest = Path(package['manifest_path'])
        if (not manifest.is_absolute() or not manifest.is_file()
                or manifest.resolve(strict=True) != manifest or not beneath(manifest, root)):
            raise ValueError('Package manifest must be an ordinary workspace file')
        identity = (package['id'], selected.kind, selected.target)
        group = groups.setdefault(identity, {'package': selected.package, 'kind': selected.kind,
                                            'target': selected.target, 'cwd': manifest.parent,
                                            'filters': [], 'all_tests': False})
        if selected.filter is None:
            group['all_tests'] = True
        else:
            group['filters'].append(selected.filter)
    executable_owners = set()
    for identity, group in groups.items():
        artifacts = [event for event in receipts if event.get('reason') == 'compiler-artifact'
                     and (event.get('package_id'), target_kind(event.get('target', {})),
                          event.get('target', {}).get('name')) == identity
                     and event.get('profile', {}).get('test') is True]
        if len(artifacts) != 1 or not isinstance(artifacts[0].get('executable'), str):
            raise ValueError(f'Missing or duplicate executable receipt for {identity}')
        executable = Path(artifacts[0]['executable'])
        pattern = re.escape(group['target'].replace('-', '_')) + r'-[0-9a-f]+(?:\.exe)?'
        if (not executable.is_absolute() or not executable.is_file()
                or executable.resolve(strict=True) != executable or not beneath(executable, target)
                or not re.fullmatch(pattern, executable.name)):
            raise ValueError(f'Unowned test executable: {executable}')
        if executable in executable_owners:
            raise ValueError('One executable cannot represent multiple package/target identities')
        executable_owners.add(executable)
        group.update(executable=executable, signature=signature(executable), sha256=digest(executable))
    return list(groups.values()), target


def parse_counts(text):
    matches = list(COUNTS.finditer(text))
    if not matches:
        return None
    return dict(zip(('passed', 'failed', 'ignored', 'measured', 'filtered'),
                    map(int, matches[-1].groups())))


class Stopped(Exception):
    pass


class Children:
    def __init__(self, spawn, timeout):
        self.spawn = spawn
        self.timeout = timeout
        self.lock = threading.Lock()
        self.active = {}
        self.stop = threading.Event()
        self.peak = 0

    def call(self, command, cwd, environment, log):
        with log.open('wb') as output:
            with self.lock:
                if self.stop.is_set():
                    raise Stopped('Admission stopped after another case failed or interruption')
                options = {'cwd': cwd, 'env': environment, 'stdin': subprocess.DEVNULL,
                           'stdout': output, 'stderr': subprocess.STDOUT}
                if os.name == 'nt':
                    options['creationflags'] = subprocess.CREATE_NO_WINDOW
                child = self.spawn(command, **options)
                self.active[child.pid] = child
                self.peak = max(self.peak, len(self.active))
            try:
                return child.wait(timeout=self.timeout)
            except subprocess.TimeoutExpired as error:
                self.terminate(child)
                raise TimeoutError(f'Test child exceeded {self.timeout:g} seconds') from error
            except BaseException:
                # wait() can fail while the launched process is still alive. Retain
                # ownership until it has exited, including on exceptional paths.
                self.terminate(child)
                raise
            finally:
                if child.poll() is not None:
                    with self.lock:
                        self.active.pop(child.pid, None)

    @staticmethod
    def terminate(child):
        if child.poll() is not None:
            return
        try:
            child.terminate()
        except OSError:
            if child.poll() is None:
                raise
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                child.kill()
            except OSError:
                if child.poll() is None:
                    raise
            child.wait(timeout=5)

    def terminate_owned(self):
        self.stop.set()
        with self.lock:
            children = list(self.active.values())
        errors = []
        for child in children:
            try:
                self.terminate(child)
            except (OSError, subprocess.SubprocessError) as error:
                errors.append(f'Owned child {child.pid}: {error}')
        return errors


def child_environment(base, directory, executable, runtime_paths, threads):
    environment = dict(base)
    temporary, render = directory / 'tmp', directory / 'render'
    temporary.mkdir(mode=0o700)
    render.mkdir(mode=0o700)
    for key in ('TEMP', 'TMP', 'TMPDIR'):
        environment[key] = str(temporary)
    environment['MIDNIGHT_UI_RENDER_DIR'] = str(render)
    environment['RUST_TEST_THREADS'] = str(threads)
    key = 'PATH' if os.name == 'nt' else ('DYLD_FALLBACK_LIBRARY_PATH' if sys.platform == 'darwin'
                                        else 'LD_LIBRARY_PATH')
    paths = list(dict.fromkeys([str(executable.parent), str(executable.parent.parent),
                               *map(str, runtime_paths)]))
    if environment.get(key):
        paths.append(environment[key])
    environment[key] = os.pathsep.join(paths)
    return environment


def execute(root, metadata, receipts, selections, log_dir, *, jobs=None, thread_budget=None,
            environment=None, spawn=subprocess.Popen, runtime_paths=(), exact=False, timeout=300):
    plans, target = prepare(Path(root), metadata, receipts, selections)
    cpus = available_cpus()
    jobs = min(4, cpus) if jobs is None else jobs
    thread_budget = cpus if thread_budget is None else thread_budget
    if type(jobs) is not int or not 1 <= jobs <= 4 or type(thread_budget) is not int or thread_budget < 1:
        raise ValueError('Jobs must be 1..4 and the total test-thread budget must be positive')
    if not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('The per-process timeout must be finite and positive')
    jobs = min(jobs, cpus, thread_budget, len(plans))
    environment = dict(os.environ if environment is None else environment)
    inherited_threads = environment.get('RUST_TEST_THREADS')
    if inherited_threads is not None and (not inherited_threads.isdecimal() or int(inherited_threads) < 1):
        raise ValueError('RUST_TEST_THREADS must be a positive integer')
    threads = max(1, thread_budget // jobs)
    if inherited_threads:
        threads = min(threads, int(inherited_threads))
    log_dir = Path(log_dir).absolute()
    build = Path(root).resolve(strict=True) / '_build'
    if log_dir.resolve() != log_dir or not beneath(log_dir, build):
        raise ValueError('Logs must remain under the ordinary workspace _build directory')
    log_dir.mkdir(parents=True, exist_ok=False, mode=0o700)
    # Cargo exposes generated native library search directories inside its target tree.
    generated = []
    for event in receipts:
        if event.get('reason') == 'build-script-executed':
            for value in event.get('linked_paths', ()):
                path = Path(value.partition('=')[2] if '=' in value else value)
                if path.is_absolute() and path.is_dir() and beneath(path.resolve(), target):
                    generated.append(path.resolve())
    runtime_paths = [*runtime_paths, *generated]
    children = Children(spawn, timeout)
    results = []
    for index, plan in enumerate(plans):
        label = re.sub(r'[^A-Za-z0-9_.-]', '_', f'{plan["package"]}-{plan["kind"]}-{plan["target"]}')
        directory = log_dir / f'{index:03d}-{label}'
        directory.mkdir(mode=0o700)
        plan['directory'] = directory
        results.append({'package': plan['package'], 'kind': plan['kind'], 'target': plan['target'],
                        'filters': plan['filters'], 'all_tests': plan['all_tests'],
                        'status': 'not-run', 'executable': str(plan['executable']),
                        'sha256': plan['sha256'], 'cwd': str(plan['cwd']),
                        'list_log': str(directory / 'list.log'), 'test_log': str(directory / 'test.log')})

    def run_case(index):
        plan, result = plans[index], results[index]
        started = time.monotonic()
        try:
            env = child_environment(environment, plan['directory'], plan['executable'], runtime_paths, threads)
            if signature(plan['executable']) != plan['signature']:
                raise ValueError('Selected executable changed after receipt validation')
            command = [str(plan['executable']), '--list', '--test', '--color', 'never']
            code = children.call(command, plan['cwd'], env, Path(result['list_log']))
            if code:
                result['exit_code'] = code
                raise ValueError(f'libtest listing failed with exit {code}')
            with Path(result['list_log']).open(encoding='utf-8', errors='replace') as stream:
                names = {line.removesuffix('\n').removesuffix('\r')[:-6]
                         for line in stream if line.rstrip('\r\n').endswith(': test')}
            counts = {value: sum(name == value if exact else value in name for name in names)
                      for value in plan['filters']}
            result['filter_matches'] = counts
            for value, count in counts.items():
                if count == 0:
                    raise ValueError(f'Explicit filter matched no tests: {value}')
            matched = names if plan['all_tests'] else {
                name for name in names if any(name == value if exact else value in name for value in plan['filters'])}
            result['matched'] = len(matched)
            if not matched:
                raise ValueError('Selected target matched no tests')
            if signature(plan['executable']) != plan['signature']:
                raise ValueError('Selected executable changed after listing')
            filters = [] if plan['all_tests'] else list(dict.fromkeys(plan['filters']))
            command = [str(plan['executable']), *filters, '--test-threads', str(threads),
                       '--test', '--color', 'never']
            if exact:
                command.append('--exact')
            result['exit_code'] = children.call(command, plan['cwd'], env, Path(result['test_log']))
            # Parse only complete anchored libtest summaries, never substring '0 passed'.
            with Path(result['test_log']).open(encoding='utf-8', errors='replace') as stream:
                counts = None
                for line in stream:
                    counts = parse_counts(line) or counts
            result['counts'] = counts
            if result['exit_code'] != 0:
                raise ValueError(f'Test process exited {result["exit_code"]}')
            if counts is None or counts['passed'] + counts['failed'] == 0:
                raise ValueError('No executed tests were reported by libtest')
            if counts['failed'] or counts['passed'] + counts['failed'] + counts['ignored'] != len(matched):
                raise ValueError('libtest results do not cover the exact listed selection')
            result['status'] = 'passed'
        except Stopped as error:
            result['error'] = str(error)
        except Exception as error:
            result.update(status='failed', error=str(error))
            children.stop.set()
        finally:
            result['elapsed_seconds'] = round(time.monotonic() - started, 3)

    interrupted = False
    cleanup_errors = []
    pool = ThreadPoolExecutor(max_workers=jobs)
    try:
        futures = [pool.submit(run_case, index) for index in range(len(plans))]
        for future in as_completed(futures):
            future.result()
    except BaseException:
        interrupted = True
        cleanup_errors = children.terminate_owned()
    finally:
        pool.shutdown(wait=True, cancel_futures=True)
        # A failed terminate/wait in a worker leaves the exact child registered.
        # Retry that retained ownership even on an ordinary test failure.
        cleanup_errors.extend(children.terminate_owned())
    code = 130 if interrupted else (0 if not cleanup_errors and all(item['status'] == 'passed' for item in results) else 1)
    summary = {'exit_code': code, 'jobs': jobs, 'thread_budget': thread_budget,
               'threads_per_process': threads, 'peak_processes': children.peak,
               'interrupted': interrupted, 'timeout_seconds': timeout,
               'cleanup_errors': cleanup_errors, 'cases': results}
    (log_dir / 'summary.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    for item in results:
        print(f'{item["status"]}: {item["package"]} {item["kind"]}:{item["target"]}'
              + (f' — {item["error"]}' if 'error' in item else ''), flush=True)
    print(f'Summary: {log_dir / "summary.json"}', flush=True)
    return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--metadata', type=Path, required=True)
    parser.add_argument('--receipt', type=Path, action='append', required=True)
    parser.add_argument('--target', nargs=2, action='append', default=[], metavar=('PACKAGE', 'KIND:TARGET'))
    parser.add_argument('--test', nargs=3, action='append', default=[], metavar=('PACKAGE', 'KIND:TARGET', 'FILTER'))
    parser.add_argument('--exact', action='store_true')
    parser.add_argument('--jobs', type=int, choices=range(1, 5))
    parser.add_argument('--thread-budget', type=int)
    parser.add_argument('--log-dir', type=Path)
    parser.add_argument('--timeout', type=float, default=300, help='Per child deadline in seconds (default: 300)')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    selections = []
    for row in [*args.target, *args.test]:
        kind, separator, target = row[1].partition(':')
        if not separator or kind not in {'lib', 'bin', 'test', 'example'} or not target:
            parser.error('Targets require lib:, bin:, test: or example: followed by the exact target name')
        selections.append(Selection(row[0], kind, target, row[2] if len(row) == 3 else None))
    metadata = json.loads(args.metadata.read_text(encoding='utf-8-sig'))
    receipts = read_receipts(args.receipt)
    runtime = Path(subprocess.check_output(['rustc', '--print', 'target-libdir'], cwd=root,
                                          text=True, encoding='utf-8', timeout=15).strip())
    if not runtime.is_absolute() or not runtime.is_dir():
        raise ValueError('The native rustc runtime library directory is unavailable')
    log_dir = args.log_dir or root / '_build/qa/targeted-tests' / (time.strftime('%Y%m%d-%H%M%S-') + uuid.uuid4().hex[:8])
    return execute(root, metadata, receipts, selections, log_dir, jobs=args.jobs,
                   thread_budget=args.thread_budget, runtime_paths=[runtime], exact=args.exact, timeout=args.timeout)


if __name__ == '__main__':
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, 'reconfigure'):
            stream.reconfigure(encoding='utf-8', errors='backslashreplace')
    try:
        sys.exit(main())
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as error:
        print(f'Targeted tests: {error}', file=sys.stderr)
        sys.exit(2)
