#!/usr/bin/env python3
"""Run the build-script tests of one folder, a test module per process, several at once.

Each `test*.py` module of the folder runs exactly as `python -B -m unittest discover -s
FOLDER -p MODULE -v` would run it, from the repository root, in a process of its own.
Modules run side by side, at most `--jobs` at a time; the tests of one module still run
one after another in one process, as they were written to. Each module's output is
printed whole when it ends, then a summary. The exit status is 1 when any module fails
or runs no test, 0 otherwise. The largest modules start first, so that a long module does
not start last.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import re
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
RAN = re.compile(r'^Ran (\d+) tests? in ', re.MULTILINE)
SKIPPED = re.compile(r'skipped=(\d+)')


def modules(folder):
    """The folder's test modules as `unittest discover` finds them, largest first.

    Discovery also enters packages below the folder; this runner refuses them instead of
    leaving their tests out."""
    packages = [path for path in folder.iterdir() if (path / '__init__.py').is_file()]
    if packages:
        raise SystemExit(f'{packages[0]}: test packages are not supported; run unittest discover')
    found = [path for path in folder.glob('test*.py') if path.is_file()]
    return sorted(found, key=lambda path: (-path.stat().st_size, path.name))


def run_module(python, folder, module):
    started = time.monotonic()
    result = subprocess.run(
        [python, '-B', '-m', 'unittest', 'discover', '-s', str(folder), '-p', module.name, '-v'],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
        # The child writes to a pipe in this interpreter's preferred encoding, as this
        # process decodes it.
        text=True, errors='replace')
    output = result.stdout
    ran = RAN.findall(output)
    tests = int(ran[-1]) if ran else 0
    tail = output[output.rfind('\nRan '):] if ran else ''
    skipped = int(SKIPPED.search(tail).group(1)) if SKIPPED.search(tail) else 0
    return {'module': module.stem, 'code': result.returncode, 'output': output, 'tests': tests,
            'skipped': skipped, 'seconds': time.monotonic() - started}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--start-directory', '-s', default='scripts/tests',
                        help='folder of test modules, relative to the repository root')
    parser.add_argument('--jobs', '-j', type=int, default=min(8, os.cpu_count() or 1),
                        help='modules run at once (default: CPUs, at most 8)')
    args = parser.parse_args(argv)
    if args.jobs < 1:
        parser.error('--jobs must be at least 1')
    folder = (ROOT / args.start_directory).resolve()
    found = modules(folder)
    if not found:
        raise SystemExit(f'{folder}: no test modules')
    lock = threading.Lock()
    started = time.monotonic()

    def report(module):
        result = run_module(sys.executable, folder, module)
        with lock:
            status = 'ok' if result['code'] == 0 and result['tests'] else 'FAILED'
            print(f"===== {result['module']}: {status}, {result['tests']} tests "
                  f"({result['skipped']} skipped) in {result['seconds']:.1f} s =====")
            print(result['output'], end='' if result['output'].endswith('\n') else '\n',
                  flush=True)
        return result

    with ThreadPoolExecutor(max_workers=min(args.jobs, len(found))) as pool:
        results = list(pool.map(report, found))
    failed = [result['module'] for result in results if result['code'] != 0 or not result['tests']]
    print(f"Ran {sum(result['tests'] for result in results)} tests "
          f"({sum(result['skipped'] for result in results)} skipped) of {len(results)} modules "
          f"in {time.monotonic() - started:.1f} s with {min(args.jobs, len(found))} at a time")
    if failed:
        print('FAILED modules: ' + ', '.join(failed))
        return 1
    print('OK')
    return 0


if __name__ == '__main__':
    sys.exit(main())
