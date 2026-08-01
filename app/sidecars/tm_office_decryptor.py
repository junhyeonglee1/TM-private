"""Memory-only OOXML decryptor used by the TM Windows expense importer.

Protocol (stdin): four-byte big-endian password length, UTF-8 password bytes,
then the encrypted Office file. Protocol (stdout): one JSON line followed by
decrypted OOXML bytes when ``ok`` is true. The process never writes a file.
"""

from __future__ import annotations

import gc
import hashlib
import io
import json
import struct
import sys

import msoffcrypto


MAX_PASSWORD_BYTES = 1_024
MAX_INPUT_BYTES = 20 * 1024 * 1024
MAX_OUTPUT_BYTES = 40 * 1024 * 1024
OLE_MAGIC = bytes.fromhex("D0CF11E0A1B11AE1")
ZIP_MAGIC = b"PK"


def respond(payload: dict[str, object], body: bytes = b"") -> None:
    header = json.dumps(payload, ensure_ascii=True, separators=(",", ":")).encode("ascii")
    sys.stdout.buffer.write(header + b"\n")
    if body:
        sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def fail(code: str, message: str) -> int:
    respond({"ok": False, "code": code, "message": message})
    return 1


def main() -> int:
    raw = sys.stdin.buffer.read(MAX_INPUT_BYTES + MAX_PASSWORD_BYTES + 5)
    if len(raw) < 4:
        return fail("invalid_request", "The decryptor request is incomplete.")

    password_length = struct.unpack(">I", raw[:4])[0]
    if password_length == 0 or password_length > MAX_PASSWORD_BYTES:
        return fail("invalid_password", "The workbook password is invalid.")

    content_offset = 4 + password_length
    if len(raw) <= content_offset:
        return fail("invalid_request", "The encrypted workbook is missing.")
    if len(raw) - content_offset > MAX_INPUT_BYTES:
        return fail("file_too_large", "The encrypted workbook exceeds the safety limit.")

    password_bytes = bytearray(raw[4:content_offset])
    encrypted = raw[content_offset:]
    raw = b""
    if not encrypted.startswith(OLE_MAGIC):
        for index in range(len(password_bytes)):
            password_bytes[index] = 0
        return fail("unsupported_encryption", "The workbook is not supported encrypted OOXML.")

    try:
        password = password_bytes.decode("utf-8", errors="strict")
    except UnicodeDecodeError:
        for index in range(len(password_bytes)):
            password_bytes[index] = 0
        return fail("invalid_password", "The workbook password is invalid.")

    output = io.BytesIO()
    try:
        office_file = msoffcrypto.OfficeFile(io.BytesIO(encrypted))
        office_file.load_key(password=password, verify_password=True)
        office_file.decrypt(output, verify_integrity=True)
        decrypted = output.getvalue()
    except Exception:
        return fail("wrong_password_or_corrupt", "The password is wrong or the workbook is corrupt.")
    finally:
        password = ""
        for index in range(len(password_bytes)):
            password_bytes[index] = 0
        output.close()
        gc.collect()

    if len(decrypted) > MAX_OUTPUT_BYTES:
        return fail("decrypted_file_too_large", "The decrypted workbook exceeds the safety limit.")
    if not decrypted.startswith(ZIP_MAGIC):
        return fail("invalid_ooxml", "The decrypted workbook is not valid OOXML.")

    respond(
        {
            "ok": True,
            "length": len(decrypted),
            "sha256": hashlib.sha256(decrypted).hexdigest(),
        },
        decrypted,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
