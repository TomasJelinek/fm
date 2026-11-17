"""A minimal unsigned x64 PE image for Windows script fixtures.

Authenticode reports it NotSigned (not UnknownError, as for arbitrary bytes), and
LoadLibraryEx can map it as an image without running any code. Bytes after the
section are overlay data, so a trailer keeps otherwise identical images distinct.
"""
import struct


def unsigned_pe(dll=False, trailer=b""):
    dos = bytearray(64)
    dos[0:2] = b"MZ"
    struct.pack_into("<I", dos, 0x3C, 0x40)
    coff = struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, 240, 0x2022 if dll else 0x0022)
    optional = struct.pack(
        "<HBBIIIIIQIIHHHHHHIIIIHHQQQQII", 0x20B, 14, 0, 0x200, 0, 0, 0x1000, 0x1000,
        0x180000000 if dll else 0x140000000, 0x1000, 0x200, 6, 0, 0, 0, 6, 0, 0, 0x2000, 0x200, 0,
        3, 0x8160, 0x100000, 0x1000, 0x100000, 0x1000, 0, 16) + bytes(128)
    section = struct.pack("<8sIIIIIIHHI", b".text", 0x200, 0x1000, 0x200, 0x200, 0, 0, 0, 0, 0x60000020)
    headers = bytes(dos) + b"PE\0\0" + coff + optional + section
    return headers.ljust(0x200, b"\0") + b"\xc3".ljust(0x200, b"\0") + trailer
