#!/usr/bin/env python3
"""Write the read-only ZIP that the installed archive worker qualification streams.

The ignored `midnight-filesystem-host` `streaming_worker` tests read it from
MIDNIGHT_ARCHIVE_TEST_ZIP. They need a source above the former 64 MiB ceiling, a
small central directory and a dated root `.exe` member that starts with "MZ". The
contents are deterministic and incompressible; an existing path is never replaced.
"""
import random
import sys
import zipfile
from pathlib import Path

PAYLOAD_BYTES = 65 * 1024 * 1024
TOOL_BYTES = 1024 * 1024
DATE = (2026, 11, 18, 12, 0, 0)


def write(path):
    generator = random.Random(64)
    with zipfile.ZipFile(path, "x") as archive:
        tool = zipfile.ZipInfo("tool.exe", date_time=DATE)
        tool.compress_type = zipfile.ZIP_DEFLATED
        archive.writestr(tool, b"MZ" + generator.randbytes(TOOL_BYTES - 2))
        archive.writestr(zipfile.ZipInfo("payload.bin", date_time=DATE), generator.randbytes(PAYLOAD_BYTES))


def main():
    if len(sys.argv) != 2:
        print("Usage: archive-test-zip.py OUTPUT.zip", file=sys.stderr)
        return 2
    path = Path(sys.argv[1])
    try:
        write(path)
    except FileExistsError:
        print(f"Refusing to replace an existing file: {path}", file=sys.stderr)
        return 1
    print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
