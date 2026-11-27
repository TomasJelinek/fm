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


def importing_pe(imports=(), delay_imports=(), *, pe32=False, legacy_delay=False):
    """A PE image whose import and delay-load tables name these DLLs (no code, no thunks).

    `pe32` builds a 32-bit (PE32) header; `legacy_delay` writes Visual C++ 6 delay-load
    descriptors, which hold addresses instead of RVAs.
    """
    if legacy_delay and not pe32:
        raise ValueError("Visual C++ 6 delay-load descriptors exist only in PE32 images")
    base = 0x400000 if pe32 else 0x140000000
    names = b""
    offsets = []
    for name in (*imports, *delay_imports):
        offsets.append(len(names))
        names += name.encode("ascii") + b"\0"
    import_size = 20 * (len(imports) + 1)
    delay_size = 32 * (len(delay_imports) + 1)
    names_rva = 0x1000 + import_size + delay_size
    data = bytearray()
    for offset in offsets[:len(imports)]:
        data += struct.pack("<IIIII", 0, 0, 0, names_rva + offset, 0)
    data += bytes(20)
    for offset in offsets[len(imports):]:
        name = names_rva + offset
        data += struct.pack("<IIIIIIII", 0 if legacy_delay else 1, base + name if legacy_delay else name,
                            0, 0, 0, 0, 0, 0)
    data += bytes(32) + names
    raw_size = (len(data) + 0x1FF) & ~0x1FF
    directories = bytearray(128)
    if imports:
        struct.pack_into("<II", directories, 8, 0x1000, import_size)
    if delay_imports:
        struct.pack_into("<II", directories, 13 * 8, 0x1000 + import_size, delay_size)
    if pe32:
        optional = struct.pack(
            "<HBBIIIIIIIIIHHHHHHIIIIHHIIIIII", 0x10B, 14, 0, 0, raw_size, 0, 0, 0x1000, 0x1000, base,
            0x1000, 0x200, 6, 0, 0, 0, 6, 0, 0, 0x2000, 0x200, 0, 3, 0x8140, 0x100000, 0x1000,
            0x100000, 0x1000, 0, 16)
    else:
        optional = struct.pack(
            "<HBBIIIIIQIIHHHHHHIIIIHHQQQQII", 0x20B, 14, 0, 0, raw_size, 0, 0, 0x1000, base,
            0x1000, 0x200, 6, 0, 0, 0, 6, 0, 0, 0x2000, 0x200, 0, 3, 0x8160, 0x100000, 0x1000,
            0x100000, 0x1000, 0, 16)
    optional += bytes(directories)
    dos = bytearray(64)
    dos[0:2] = b"MZ"
    struct.pack_into("<I", dos, 0x3C, 0x40)
    coff = struct.pack("<HHIIIHH", 0x14C if pe32 else 0x8664, 1, 0, 0, 0, len(optional), 0x2102 if pe32 else 0x0022)
    section = struct.pack("<8sIIIIIIHHI", b".rdata", raw_size, 0x1000, raw_size, 0x200, 0, 0, 0, 0, 0x40000040)
    headers = bytes(dos) + b"PE\0\0" + coff + optional + section
    return headers.ljust(0x200, b"\0") + bytes(data).ljust(raw_size, b"\0")
