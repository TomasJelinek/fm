#!/usr/bin/env python3
"""Sign a release's SHA256SUMS for the update check (docs/developer/updates.md).

Commands:
  generate-key --secret-key PATH   once: a new Ed25519 key pair in minisign format; the
                                   secret key is written OUTSIDE the repository and the
                                   public key into packaging/update-signing.pub
  checksums ARCHIVE...             write SHA256SUMS beside the archives
  sign --secret-key PATH --version X.Y.Z SHA256SUMS
                                   write SHA256SUMS.minisig, then verify it
  verify --version X.Y.Z SHA256SUMS
                                   check SHA256SUMS.minisig with the repository's key

Python 3.11+ standard library only: Ed25519 (RFC 8032) is implemented below, BLAKE2b
and scrypt come from hashlib. The files are compatible with minisign 0.11 (pre-hashed
signatures; scrypt-encrypted secret keys with libsodium's "sensitive" limits).
"""

import argparse
import base64
import getpass
import hashlib
import os
import secrets
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PUBLIC_KEY = ROOT / "packaging" / "update-signing.pub"
PLACEHOLDER = "PLACEHOLDER"

# --- Ed25519, RFC 8032 section 5.1 --------------------------------------------------
# Not constant-time: run it only on the owner's own machine, where the key is used.
P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, P - 2, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def _recover_x(y, sign):
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


_BY = 4 * pow(5, P - 2, P) % P
_BX = _recover_x(_BY, 0)
BASE = (_BX, _BY, 1, _BX * _BY % P)


def _add(a, b):
    p1 = (a[1] - a[0]) * (b[1] - b[0]) % P
    p2 = (a[1] + a[0]) * (b[1] + b[0]) % P
    c = 2 * a[3] * b[3] * D % P
    d = 2 * a[2] * b[2] % P
    e, f, g, h = p2 - p1, d - c, d + c, p2 + p1
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def _multiply(scalar, point):
    result = (0, 1, 1, 0)
    while scalar:
        if scalar & 1:
            result = _add(result, point)
        point = _add(point, point)
        scalar >>= 1
    return result


def _encode(point):
    z = pow(point[2], P - 2, P)
    x, y = point[0] * z % P, point[1] * z % P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def _decode(data):
    if len(data) != 32:
        return None
    y = int.from_bytes(data, "little")
    sign = y >> 255
    y &= (1 << 255) - 1
    x = _recover_x(y, sign)
    return None if x is None else (x, y, 1, x * y % P)


def _equal(a, b):
    return (a[0] * b[2] - b[0] * a[2]) % P == 0 and (a[1] * b[2] - b[1] * a[2]) % P == 0


def _expand(seed):
    digest = hashlib.sha512(seed).digest()
    scalar = int.from_bytes(digest[:32], "little")
    scalar &= (1 << 254) - 8
    scalar |= 1 << 254
    return scalar, digest[32:]


def ed25519_public(seed):
    return _encode(_multiply(_expand(seed)[0], BASE))


def ed25519_sign(seed, message):
    scalar, prefix = _expand(seed)
    public = _encode(_multiply(scalar, BASE))
    r = int.from_bytes(hashlib.sha512(prefix + message).digest(), "little") % L
    big_r = _encode(_multiply(r, BASE))
    h = int.from_bytes(hashlib.sha512(big_r + public + message).digest(), "little") % L
    return big_r + ((r + h * scalar) % L).to_bytes(32, "little")


def ed25519_verify(public, message, signature):
    if len(signature) != 64:
        return False
    point_a, point_r = _decode(public), _decode(signature[:32])
    s = int.from_bytes(signature[32:], "little")
    if point_a is None or point_r is None or s >= L:
        return False
    h = int.from_bytes(hashlib.sha512(signature[:32] + public + message).digest(), "little") % L
    return _equal(_multiply(s, BASE), _add(point_r, _multiply(h, point_a)))


# --- minisign formats -----------------------------------------------------------------
# libsodium's crypto_pwhash_scryptsalsa208sha256 "sensitive" limits, minisign's default.
OPSLIMIT = 33554432
MEMLIMIT = 1073741824


def scrypt_parameters(opslimit, memlimit):
    """libsodium's pickparams: (log2 N, r, p) for the limits stored in the key."""
    opslimit = max(opslimit, 32768)
    r = 8
    if opslimit < memlimit // 32:
        p = 1
        max_n = opslimit // (r * 4)
    else:
        max_n = memlimit // (r * 128)
    log_n = 1
    while log_n < 63 and (1 << log_n) <= max_n // 2:
        log_n += 1
    if opslimit >= memlimit // 32:
        max_rp = min((opslimit // 4) // (1 << log_n), 0x3FFFFFFF)
        p = max_rp // r
    return log_n, r, p


def _stream(password, salt, opslimit, memlimit, length):
    log_n, r, p = scrypt_parameters(opslimit, memlimit)
    return hashlib.scrypt(password, salt=salt, n=1 << log_n, r=r, p=p,
                          maxmem=128 * r * (1 << log_n) + 64 * 1024 * 1024, dklen=length)


def _checksum(key_id, secret):
    return hashlib.blake2b(b"Ed" + key_id + secret, digest_size=32).digest()


def secret_key_text(seed, key_id, password, opslimit=OPSLIMIT, memlimit=MEMLIMIT):
    secret = seed + ed25519_public(seed)
    keynum = key_id + secret + _checksum(key_id, secret)
    if password is None:
        kdf, salt, opslimit, memlimit = b"\0\0", bytes(32), 0, 0
    else:
        kdf, salt = b"Sc", secrets.token_bytes(32)
        stream = _stream(password, salt, opslimit, memlimit, len(keynum))
        keynum = bytes(a ^ b for a, b in zip(keynum, stream))
    blob = (b"Ed" + kdf + b"B2" + salt + opslimit.to_bytes(8, "little")
            + memlimit.to_bytes(8, "little") + keynum)
    comment = "encrypted secret key" if password is not None else "unencrypted secret key"
    return f"untrusted comment: minisign {comment}\n{base64.b64encode(blob).decode()}\n"


def read_secret_key(text, password_prompt):
    """Returns (seed, key_id); asks for the password when the key is encrypted."""
    lines = text.splitlines()
    if len(lines) < 2:
        raise ValueError("Not a minisign secret key")
    blob = base64.b64decode(lines[1].strip(), validate=True)
    if len(blob) != 158 or blob[:2] != b"Ed" or blob[4:6] != b"B2":
        raise ValueError("Not an Ed25519 minisign secret key")
    kdf, salt = blob[2:4], blob[6:38]
    opslimit = int.from_bytes(blob[38:46], "little")
    memlimit = int.from_bytes(blob[46:54], "little")
    keynum = blob[54:]
    if kdf == b"Sc":
        stream = _stream(password_prompt(), salt, opslimit, memlimit, len(keynum))
        keynum = bytes(a ^ b for a, b in zip(keynum, stream))
    elif kdf != b"\0\0":
        raise ValueError("Unknown key derivation in the secret key")
    key_id, secret, checksum = keynum[:8], keynum[8:72], keynum[72:]
    if _checksum(key_id, secret) != checksum:
        raise ValueError("Wrong password, or the secret key is damaged")
    seed = secret[:32]
    if ed25519_public(seed) != secret[32:]:
        raise ValueError("The secret key is damaged")
    return seed, key_id


def public_key_text(seed, key_id):
    blob = b"Ed" + key_id + ed25519_public(seed)
    number = int.from_bytes(key_id, "little")
    return (f"untrusted comment: minisign public key {number:016X}\n"
            f"{base64.b64encode(blob).decode()}\n")


def read_public_key(text):
    """Returns (key_id, public key bytes), or None for the placeholder."""
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("untrusted comment:"):
            continue
        if line.startswith("#"):
            return None
        blob = base64.b64decode(line, validate=True)
        if len(blob) != 42 or blob[:2] != b"Ed":
            raise ValueError("Not an Ed25519 minisign public key")
        return blob[2:10], blob[10:]
    return None


def release_marker(version):
    return f"release:midnight-explorer-{version}"


def signature_text(seed, key_id, data, trusted_comment):
    signature = ed25519_sign(seed, hashlib.blake2b(data, digest_size=64).digest())
    global_signature = ed25519_sign(seed, signature + trusted_comment.encode())
    return ("untrusted comment: signature from the Midnight Explorer release key\n"
            f"{base64.b64encode(b'ED' + key_id + signature).decode()}\n"
            f"trusted comment: {trusted_comment}\n"
            f"{base64.b64encode(global_signature).decode()}\n")


def verify_signature(public, data, text, version):
    """Raises ValueError unless `text` is a valid pre-hashed signature of `data` by
    `public` whose trusted comment names `version`."""
    key_id, key = public
    lines = text.splitlines()
    if len(lines) < 4 or not lines[2].startswith("trusted comment: "):
        raise ValueError("Not a minisign signature")
    first = base64.b64decode(lines[1].strip(), validate=True)
    global_signature = base64.b64decode(lines[3].strip(), validate=True)
    if len(first) != 74 or first[:2] != b"ED":
        raise ValueError("Not a pre-hashed Ed25519 minisign signature")
    if first[2:10] != key_id:
        raise ValueError("Signed with another key")
    signature = first[10:]
    comment = lines[2][len("trusted comment: "):]
    if not ed25519_verify(key, hashlib.blake2b(data, digest_size=64).digest(), signature):
        raise ValueError("The signature does not match the file")
    if not ed25519_verify(key, signature + comment.encode(), global_signature):
        raise ValueError("The trusted comment was changed")
    if release_marker(version) not in comment.split():
        raise ValueError(f"The signature is not for version {version}")


# --- commands -------------------------------------------------------------------------
def outside_repository(path):
    resolved = Path(os.path.abspath(path))
    try:
        resolved = resolved.parent.resolve() / resolved.name
    except OSError:
        pass
    try:
        resolved.relative_to(ROOT)
    except ValueError:
        return resolved
    raise SystemExit(f"Refused: {resolved} is inside the repository. Keep the secret key "
                     "outside every checkout, for example in your home folder.")


def ask_new_password():
    first = getpass.getpass("Password for the new secret key: ")
    if len(first) < 12:
        raise SystemExit("Use a password of at least 12 characters.")
    if getpass.getpass("Repeat the password: ") != first:
        raise SystemExit("The passwords differ.")
    return first.encode()


def command_generate(args, out=sys.stdout):
    secret_path = outside_repository(args.secret_key)
    public_path = Path(args.public_key) if args.public_key else PUBLIC_KEY
    if secret_path.exists():
        raise SystemExit(f"Refused: {secret_path} exists. A key is never overwritten.")
    if public_path.exists() and PLACEHOLDER not in public_path.read_text(encoding="utf-8"):
        raise SystemExit(f"Refused: {public_path} already holds a public key.")
    password = None if args.no_password else ask_new_password()
    seed, key_id = secrets.token_bytes(32), secrets.token_bytes(8)
    secret_path.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0)
    descriptor = os.open(secret_path, flags, 0o600)
    with os.fdopen(descriptor, "w", encoding="ascii", newline="\n") as stream:
        stream.write(secret_key_text(seed, key_id, password))
    public_path.write_text(public_key_text(seed, key_id), encoding="ascii", newline="\n")
    print(f"Secret key written to {secret_path}", file=out)
    print("  Keep it (and its password) private and backed up; it never enters the "
          "repository. Losing it means releases can no longer update older copies.", file=out)
    print(f"Public key written to {public_path}", file=out)
    print("  Commit it: builds made from now on trust only releases signed with this key.",
          file=out)


def command_checksums(args, out=sys.stdout):
    archives = [Path(name) for name in args.archives]
    folder = archives[0].resolve().parent
    if any(archive.resolve().parent != folder for archive in archives):
        raise SystemExit("All archives must be in one folder.")
    output = folder / "SHA256SUMS"
    if output.exists() and not args.force:
        raise SystemExit(f"Refused: {output} exists (use --force to replace it).")
    lines = []
    for archive in sorted(archives, key=lambda path: path.name):
        with archive.open("rb") as stream:
            lines.append(f"{hashlib.file_digest(stream, 'sha256').hexdigest()}  {archive.name}\n")
    output.write_text("".join(lines), encoding="ascii", newline="\n")
    print(output, file=out)


def command_sign(args, out=sys.stdout):
    secret_path = outside_repository(args.secret_key)
    sums = Path(args.sums)
    signature_path = sums.with_name(sums.name + ".minisig")
    if signature_path.exists() and not args.force:
        raise SystemExit(f"Refused: {signature_path} exists (use --force to replace it).")
    data = sums.read_bytes()
    seed, key_id = read_secret_key(
        secret_path.read_text(encoding="ascii"),
        lambda: getpass.getpass("Password of the secret key: ").encode())
    comment = f"timestamp:{int(time.time())} file:{sums.name} {release_marker(args.version)}"
    text = signature_text(seed, key_id, data, comment)
    verify_signature((key_id, ed25519_public(seed)), data, text, args.version)
    public_path = Path(args.public_key) if args.public_key else PUBLIC_KEY
    public = read_public_key(public_path.read_text(encoding="utf-8"))
    if public is None:
        print(f"Warning: {public_path} is the placeholder; builds trust no key yet.", file=out)
    elif public != (key_id, ed25519_public(seed)):
        raise SystemExit(f"Refused: the secret key does not belong to {public_path}.")
    signature_path.write_text(text, encoding="ascii", newline="\n")
    print(signature_path, file=out)


def command_verify(args, out=sys.stdout):
    sums = Path(args.sums)
    public_path = Path(args.public_key) if args.public_key else PUBLIC_KEY
    public = read_public_key(public_path.read_text(encoding="utf-8"))
    if public is None:
        raise SystemExit(f"{public_path} holds no key.")
    signature = sums.with_name(sums.name + ".minisig").read_text(encoding="ascii")
    try:
        verify_signature(public, sums.read_bytes(), signature, args.version)
    except ValueError as error:
        raise SystemExit(f"Signature refused: {error}")
    print(f"{sums} is signed for version {args.version}.", file=out)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    generate = commands.add_parser("generate-key")
    generate.add_argument("--secret-key", required=True)
    generate.add_argument("--public-key", help="default: packaging/update-signing.pub")
    generate.add_argument("--no-password", action="store_true",
                          help="store the secret key unencrypted (tests only)")
    generate.set_defaults(run=command_generate)
    checksums = commands.add_parser("checksums")
    checksums.add_argument("archives", nargs="+")
    checksums.add_argument("--force", action="store_true")
    checksums.set_defaults(run=command_checksums)
    sign = commands.add_parser("sign")
    sign.add_argument("--secret-key", required=True)
    sign.add_argument("--version", required=True)
    sign.add_argument("--public-key", help="default: packaging/update-signing.pub")
    sign.add_argument("--force", action="store_true")
    sign.add_argument("sums")
    sign.set_defaults(run=command_sign)
    verify = commands.add_parser("verify")
    verify.add_argument("--version", required=True)
    verify.add_argument("--public-key", help="default: packaging/update-signing.pub")
    verify.add_argument("sums")
    verify.set_defaults(run=command_verify)
    args = parser.parse_args(argv)
    args.run(args)


if __name__ == "__main__":
    main()
