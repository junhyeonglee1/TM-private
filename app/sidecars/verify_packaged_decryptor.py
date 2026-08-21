"""Verify the packaged TM decryptor with a synthetic Agile SHA-1 workbook."""

from __future__ import annotations

import json
import struct
import subprocess
import sys
from pathlib import Path

from test_tm_office_decryptor import PASSWORD, encrypt_xlsx_sha1, synthetic_xlsx


def invoke(
    executable: Path,
    encrypted: bytes,
    password: str,
) -> tuple[int, dict[str, object], bytes, bytes]:
    password_bytes = password.encode("utf-8")
    request = struct.pack(">I", len(password_bytes)) + password_bytes + encrypted
    result = subprocess.run(
        [str(executable)],
        input=request,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=30,
    )
    header_bytes, separator, body = result.stdout.partition(b"\n")
    if not separator:
        raise RuntimeError("The packaged decryptor returned no JSON header.")
    return result.returncode, json.loads(header_bytes), body, result.stderr


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def main() -> int:
    if len(sys.argv) != 2:
        raise RuntimeError("Expected the packaged decryptor path.")
    executable = Path(sys.argv[1]).resolve(strict=True)
    plaintext = synthetic_xlsx()

    returncode, header, body, stderr = invoke(
        executable,
        encrypt_xlsx_sha1(plaintext),
        PASSWORD,
    )
    require(returncode == 0, "Packaged Agile SHA-1 decryption failed.")
    require(header.get("ok") is True, "Packaged decryptor did not report success.")
    require(body == plaintext, "Packaged decryptor changed the synthetic workbook.")
    require(stderr == b"", "Packaged decryptor wrote to stderr.")

    returncode, header, body, stderr = invoke(
        executable,
        encrypt_xlsx_sha1(plaintext),
        "synthetic-wrong-password",
    )
    require(returncode != 0, "Packaged decryptor accepted a wrong password.")
    require(header.get("code") == "wrong_password", "Wrong password code changed.")
    require(body == b"" and stderr == b"", "Wrong password leaked output.")

    returncode, header, body, stderr = invoke(
        executable,
        encrypt_xlsx_sha1(plaintext, corrupt_payload=True),
        PASSWORD,
    )
    require(returncode != 0, "Packaged decryptor accepted changed ciphertext.")
    require(header.get("code") == "integrity_failed", "Integrity code changed.")
    require(body == b"" and stderr == b"", "Integrity failure leaked output.")
    print("Packaged Agile SHA-1 decryptor verification passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
