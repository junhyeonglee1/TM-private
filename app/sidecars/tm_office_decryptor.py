"""Memory-only OOXML decryptor used by the TM Windows expense importer.

Protocol (stdin): four-byte big-endian password length, UTF-8 password bytes,
then the encrypted Office file. Protocol (stdout): one JSON line followed by
decrypted OOXML bytes when ``ok`` is true. The process never writes a file.
"""

from __future__ import annotations

import base64
import binascii
import gc
import hashlib
import hmac
import io
import json
import struct
import sys
import xml.etree.ElementTree as ET

import msoffcrypto
import olefile
from msoffcrypto.exceptions import DecryptionError, FileFormatError, InvalidKeyError, ParseError
from msoffcrypto.method.ecma376_agile import (
    ECMA376Agile,
    _decrypt_aes_cbc,
    _get_hash_func,
    _normalize_key,
    blkKey_VerifierHashInput,
    blkKey_dataIntegrity1,
    blkKey_dataIntegrity2,
    blkKey_encryptedVerifierHashValue,
)


MAX_PASSWORD_BYTES = 1_024
MAX_INPUT_BYTES = 20 * 1024 * 1024
MAX_OUTPUT_BYTES = 40 * 1024 * 1024
OLE_MAGIC = bytes.fromhex("D0CF11E0A1B11AE1")
ZIP_MAGIC = b"PK"
AGILE_SHA1 = "SHA1"
AGILE_AES_BLOCK_SIZE = 16
AGILE_SHA1_DIGEST_SIZE = 20
AGILE_SHA1_PADDED_SIZE = 32
AGILE_AES_KEY_BITS = 128
MAX_ENCRYPTION_INFO_BYTES = 256 * 1024
ENCRYPTION_NAMESPACE = "http://schemas.microsoft.com/office/2006/encryption"
PASSWORD_NAMESPACE = (
    "http://schemas.microsoft.com/office/2006/keyEncryptor/password"
)
PASSWORD_KEY_ENCRYPTOR_URI = PASSWORD_NAMESPACE
AGILE_ALLOWED_PROFILES = frozenset(
    {
        ("SHA1", 20, 128),
        ("SHA512", 64, 256),
    }
)


def respond(payload: dict[str, object], body: bytes = b"") -> None:
    header = json.dumps(payload, ensure_ascii=True, separators=(",", ":")).encode("ascii")
    sys.stdout.buffer.write(header + b"\n")
    if body:
        sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def fail(code: str, message: str) -> int:
    respond({"ok": False, "code": code, "message": message})
    return 1


def _matches_zero_padded_digest(actual: bytes, padded: bytes, block_size: int) -> bool:
    if block_size != AGILE_AES_BLOCK_SIZE or len(actual) != AGILE_SHA1_DIGEST_SIZE:
        return False
    if len(padded) != AGILE_SHA1_PADDED_SIZE:
        return False
    expected = actual + (b"\0" * (AGILE_SHA1_PADDED_SIZE - len(actual)))
    return hmac.compare_digest(expected, padded)


def _decode_exact_base64(
    attributes: dict[str, str],
    name: str,
    expected_length: int,
) -> bool:
    encoded = attributes.get(name)
    if not encoded or len(encoded) > 4_096:
        return False
    try:
        decoded = base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError):
        return False
    return len(decoded) == expected_length


def _preflight_encryption_info(encrypted: bytes) -> str | None:
    encryption_info = b""
    office_container = None
    try:
        office_container = olefile.OleFileIO(io.BytesIO(encrypted))
        with office_container.openstream("EncryptionInfo") as stream:
            encryption_info = stream.read(MAX_ENCRYPTION_INFO_BYTES + 1)
    except Exception:
        return None
    finally:
        if office_container is not None:
            try:
                office_container.close()
            except Exception:
                encryption_info = b""

    if len(encryption_info) < 9 or len(encryption_info) > MAX_ENCRYPTION_INFO_BYTES:
        return None
    version_major, version_minor, flags = struct.unpack("<HHI", encryption_info[:8])
    if (version_major, version_minor) in {(2, 2), (3, 2), (4, 2)}:
        return "standard"
    if (version_major, version_minor, flags) != (4, 4, 0x40):
        return None
    try:
        descriptor = encryption_info[8:].decode("utf-8", errors="strict")
    except UnicodeDecodeError:
        return None
    if "<!" in descriptor.upper() or "\x00" in descriptor:
        return None
    try:
        root = ET.fromstring(descriptor)
    except (ET.ParseError, LookupError, ValueError):
        return None
    if root.tag != f"{{{ENCRYPTION_NAMESPACE}}}encryption":
        return None
    if root.attrib:
        return None
    expected_root_tags = [
        f"{{{ENCRYPTION_NAMESPACE}}}keyData",
        f"{{{ENCRYPTION_NAMESPACE}}}dataIntegrity",
        f"{{{ENCRYPTION_NAMESPACE}}}keyEncryptors",
    ]
    root_children = list(root)
    if [child.tag for child in root_children] != expected_root_tags:
        return None
    key_data_node, integrity_node, key_encryptors_node = root_children
    if key_encryptors_node.attrib or len(key_encryptors_node) != 1:
        return None
    key_encryptor_node = key_encryptors_node[0]
    if (
        key_encryptor_node.tag
        != f"{{{ENCRYPTION_NAMESPACE}}}keyEncryptor"
        or key_encryptor_node.attrib != {"uri": PASSWORD_KEY_ENCRYPTOR_URI}
        or len(key_encryptor_node) != 1
    ):
        return None
    password_node = key_encryptor_node[0]
    if password_node.tag != f"{{{PASSWORD_NAMESPACE}}}encryptedKey":
        return None
    if any(list(node) for node in (key_data_node, integrity_node, password_node)):
        return None
    if any(
        (node.text is not None and node.text.strip())
        or (node.tail is not None and node.tail.strip())
        for node in root.iter()
    ):
        return None

    key_data = key_data_node.attrib
    required_key_data_fields = {
        "saltSize",
        "blockSize",
        "keyBits",
        "hashSize",
        "cipherAlgorithm",
        "cipherChaining",
        "hashAlgorithm",
        "saltValue",
    }
    if set(key_data) != required_key_data_fields:
        return None
    try:
        key_data_profile = (
            key_data["hashAlgorithm"],
            int(key_data["hashSize"]),
            int(key_data["keyBits"]),
        )
    except ValueError:
        return None
    if key_data_profile not in AGILE_ALLOWED_PROFILES or key_data != {
        "saltSize": "16",
        "blockSize": "16",
        "keyBits": str(key_data_profile[2]),
        "hashSize": str(key_data_profile[1]),
        "cipherAlgorithm": "AES",
        "cipherChaining": "ChainingModeCBC",
        "hashAlgorithm": key_data_profile[0],
        "saltValue": key_data["saltValue"],
    } or not _decode_exact_base64(key_data, "saltValue", 16):
        return None

    integrity = integrity_node.attrib
    padded_key_data_hash_size = (
        (key_data_profile[1] + AGILE_AES_BLOCK_SIZE - 1) // AGILE_AES_BLOCK_SIZE
    ) * AGILE_AES_BLOCK_SIZE
    if set(integrity) != {"encryptedHmacKey", "encryptedHmacValue"} or not all(
        _decode_exact_base64(integrity, name, padded_key_data_hash_size)
        for name in ("encryptedHmacKey", "encryptedHmacValue")
    ):
        return None

    password_key = password_node.attrib
    required_password_fields = {
        "spinCount",
        "saltSize",
        "blockSize",
        "keyBits",
        "hashSize",
        "cipherAlgorithm",
        "cipherChaining",
        "hashAlgorithm",
        "saltValue",
        "encryptedVerifierHashInput",
        "encryptedVerifierHashValue",
        "encryptedKeyValue",
    }
    if set(password_key) != required_password_fields:
        return None
    dynamic_password_fields = {
        name: password_key.get(name, "")
        for name in (
            "saltValue",
            "encryptedVerifierHashInput",
            "encryptedVerifierHashValue",
            "encryptedKeyValue",
        )
    }
    try:
        spin_count = int(password_key.get("spinCount", ""))
        password_profile = (
            password_key["hashAlgorithm"],
            int(password_key["hashSize"]),
            int(password_key["keyBits"]),
        )
    except ValueError:
        return None
    if password_profile not in AGILE_ALLOWED_PROFILES:
        return None
    expected_password_key = {
        "spinCount": str(spin_count),
        "saltSize": "16",
        "blockSize": "16",
        "keyBits": str(password_profile[2]),
        "hashSize": str(password_profile[1]),
        "cipherAlgorithm": "AES",
        "cipherChaining": "ChainingModeCBC",
        "hashAlgorithm": password_profile[0],
        **dynamic_password_fields,
    }
    padded_password_hash_size = (
        (password_profile[1] + AGILE_AES_BLOCK_SIZE - 1) // AGILE_AES_BLOCK_SIZE
    ) * AGILE_AES_BLOCK_SIZE
    encrypted_key_size = (
        (password_profile[2] // 8 + AGILE_AES_BLOCK_SIZE - 1)
        // AGILE_AES_BLOCK_SIZE
    ) * AGILE_AES_BLOCK_SIZE
    if not (
        0 < spin_count <= 10_000_000
        and password_key == expected_password_key
        and _decode_exact_base64(password_key, "saltValue", 16)
        and _decode_exact_base64(
            password_key,
            "encryptedVerifierHashInput",
            AGILE_AES_BLOCK_SIZE,
        )
        and _decode_exact_base64(
            password_key,
            "encryptedVerifierHashValue",
            padded_password_hash_size,
        )
        and _decode_exact_base64(
            password_key,
            "encryptedKeyValue",
            encrypted_key_size,
        )
    ):
        return None
    if key_data_profile == (AGILE_SHA1, AGILE_SHA1_DIGEST_SIZE, AGILE_AES_KEY_BITS):
        if password_profile != key_data_profile:
            return None
        return "agile_sha1_aes128"
    if AGILE_SHA1 in {key_data_profile[0], password_profile[0]}:
        return None
    return "agile_other"


def _has_exact_agile_sha1_aes128_metadata(encrypted: bytes) -> bool:
    return _preflight_encryption_info(encrypted) == "agile_sha1_aes128"


def _is_supported_agile_sha1_password_info(info: dict[str, object]) -> bool:
    return (
        info.get("passwordHashAlgorithm") == AGILE_SHA1
        and isinstance(info.get("passwordKeyBits"), int)
        and info["passwordKeyBits"] == AGILE_AES_KEY_BITS
        and isinstance(info.get("spinValue"), int)
        and 0 < int(info["spinValue"]) <= 10_000_000
        and isinstance(info.get("passwordSalt"), bytes)
        and len(info["passwordSalt"]) == AGILE_AES_BLOCK_SIZE
        and isinstance(info.get("encryptedVerifierHashInput"), bytes)
        and len(info["encryptedVerifierHashInput"]) == AGILE_AES_BLOCK_SIZE
        and isinstance(info.get("encryptedVerifierHashValue"), bytes)
        and len(info["encryptedVerifierHashValue"]) == AGILE_SHA1_PADDED_SIZE
    )


def _verify_agile_sha1_password(info: dict[str, object], password: str) -> bool:
    if not _is_supported_agile_sha1_password_info(info):
        raise ValueError("Unsupported Agile SHA-1 password metadata")

    password_salt = info["passwordSalt"]
    hash_algorithm = str(info["passwordHashAlgorithm"])
    spin_value = int(info["spinValue"])
    key_bits = int(info["passwordKeyBits"])
    encrypted_input = info["encryptedVerifierHashInput"]
    encrypted_hash = info["encryptedVerifierHashValue"]

    iterated = ECMA376Agile._derive_iterated_hash_from_password(
        password,
        password_salt,
        hash_algorithm,
        spin_value,
    )
    input_key = ECMA376Agile._derive_encryption_key(
        iterated.digest(),
        blkKey_VerifierHashInput,
        hash_algorithm,
        key_bits,
    )
    hash_key = ECMA376Agile._derive_encryption_key(
        iterated.digest(),
        blkKey_encryptedVerifierHashValue,
        hash_algorithm,
        key_bits,
    )
    verifier_input = _decrypt_aes_cbc(encrypted_input, input_key, password_salt)
    actual_hash = _get_hash_func(hash_algorithm)(verifier_input).digest()
    expected_hash = _decrypt_aes_cbc(encrypted_hash, hash_key, password_salt)
    return _matches_zero_padded_digest(actual_hash, expected_hash, AGILE_AES_BLOCK_SIZE)


def _is_supported_agile_sha1_integrity_info(info: dict[str, object]) -> bool:
    return (
        info.get("keyDataHashAlgorithm") == AGILE_SHA1
        and info.get("keyDataBlockSize") == AGILE_AES_BLOCK_SIZE
        and isinstance(info.get("keyDataSalt"), bytes)
        and len(info["keyDataSalt"]) == AGILE_AES_BLOCK_SIZE
        and isinstance(info.get("encryptedHmacKey"), bytes)
        and len(info["encryptedHmacKey"]) == AGILE_SHA1_PADDED_SIZE
        and isinstance(info.get("encryptedHmacValue"), bytes)
        and len(info["encryptedHmacValue"]) == AGILE_SHA1_PADDED_SIZE
    )


def _verify_agile_sha1_integrity(office_file: object) -> bool:
    info = office_file.info
    if not _is_supported_agile_sha1_integrity_info(info):
        raise ValueError("Unsupported Agile SHA-1 integrity metadata")
    if not isinstance(office_file.secret_key, bytes):
        raise ValueError("The Agile secret key is unavailable")

    hash_algorithm = str(info["keyDataHashAlgorithm"])
    hash_function = _get_hash_func(hash_algorithm)
    block_size = int(info["keyDataBlockSize"])
    salt = info["keyDataSalt"]
    first_iv = _normalize_key(
        hash_function(salt + blkKey_dataIntegrity1).digest(),
        block_size,
    )
    second_iv = _normalize_key(
        hash_function(salt + blkKey_dataIntegrity2).digest(),
        block_size,
    )
    padded_hmac_key = _decrypt_aes_cbc(
        info["encryptedHmacKey"],
        office_file.secret_key,
        first_iv,
    )
    if len(padded_hmac_key) != AGILE_SHA1_PADDED_SIZE or not hmac.compare_digest(
        padded_hmac_key[AGILE_SHA1_DIGEST_SIZE:],
        b"\0" * (AGILE_SHA1_PADDED_SIZE - AGILE_SHA1_DIGEST_SIZE),
    ):
        return False
    hmac_key = padded_hmac_key[:AGILE_SHA1_DIGEST_SIZE]
    expected_hmac = _decrypt_aes_cbc(
        info["encryptedHmacValue"],
        office_file.secret_key,
        second_iv,
    )
    with office_file.file.openstream("EncryptedPackage") as encrypted_package:
        actual_hmac = hmac.new(hmac_key, encrypted_package.read(), hash_function).digest()
    return _matches_zero_padded_digest(actual_hmac, expected_hmac, block_size)


def _load_key_with_verified_password(
    office_file: object,
    password: str,
    allow_agile_sha1_compatibility: bool = False,
) -> None:
    info = getattr(office_file, "info", None)
    if (
        getattr(office_file, "type", None) == "agile"
        and isinstance(info, dict)
        and info.get("passwordHashAlgorithm") == AGILE_SHA1
    ):
        if not allow_agile_sha1_compatibility:
            raise ValueError("Agile SHA-1 compatibility metadata was not approved")
        if not _verify_agile_sha1_password(info, password):
            raise InvalidKeyError("Key verification failed")
        office_file.load_key(password=password, verify_password=False)
        return
    office_file.load_key(password=password, verify_password=True)


def _decrypt_with_verified_integrity(
    office_file: object,
    output: io.BytesIO,
    allow_agile_sha1_compatibility: bool = False,
) -> None:
    info = getattr(office_file, "info", None)
    if (
        getattr(office_file, "type", None) == "agile"
        and isinstance(info, dict)
        and info.get("keyDataHashAlgorithm") == AGILE_SHA1
    ):
        if not allow_agile_sha1_compatibility:
            raise ValueError("Agile SHA-1 compatibility metadata was not approved")
        if not _verify_agile_sha1_integrity(office_file):
            raise InvalidKeyError("Payload integrity verification failed")
        office_file.decrypt(output, verify_integrity=False)
        return
    office_file.decrypt(output, verify_integrity=True)


def decrypt_office_file(encrypted: bytes, password: str) -> tuple[str | None, bytes]:
    encryption_profile = _preflight_encryption_info(encrypted)
    if encryption_profile is None:
        return "unsupported_encryption", b""
    try:
        office_file = msoffcrypto.OfficeFile(io.BytesIO(encrypted))
    except (FileFormatError, ParseError, ValueError):
        return "unsupported_encryption", b""
    except Exception:
        return "decryptor_internal", b""

    info = getattr(office_file, "info", None)
    office_type = getattr(office_file, "type", None)
    if (
        (encryption_profile == "standard" and office_type != "standard")
        or (encryption_profile != "standard" and office_type != "agile")
        or not isinstance(info, dict)
    ):
        return "unsupported_encryption", b""
    allow_agile_sha1_compatibility = encryption_profile == "agile_sha1_aes128"
    if allow_agile_sha1_compatibility and not (
        info.get("passwordHashAlgorithm") == AGILE_SHA1
        and info.get("keyDataHashAlgorithm") == AGILE_SHA1
        and info.get("passwordKeyBits") == AGILE_AES_KEY_BITS
        and info.get("keyDataBlockSize") == AGILE_AES_BLOCK_SIZE
    ):
        return "unsupported_encryption", b""
    if encryption_profile == "agile_other" and (
        info.get("passwordHashAlgorithm") == AGILE_SHA1
        or info.get("keyDataHashAlgorithm") == AGILE_SHA1
    ):
        return "unsupported_encryption", b""

    try:
        _load_key_with_verified_password(
            office_file,
            password,
            allow_agile_sha1_compatibility,
        )
    except InvalidKeyError:
        return "wrong_password", b""
    except (FileFormatError, ParseError, DecryptionError, ValueError):
        return "key_load_failed", b""
    except Exception:
        return "decryptor_internal", b""

    output = io.BytesIO()
    try:
        _decrypt_with_verified_integrity(
            office_file,
            output,
            allow_agile_sha1_compatibility,
        )
        return None, output.getvalue()
    except InvalidKeyError:
        return "integrity_failed", b""
    except (FileFormatError, ParseError, DecryptionError, ValueError):
        return "decrypt_failed", b""
    except Exception:
        return "decryptor_internal", b""
    finally:
        output.close()


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

    try:
        error_code, decrypted = decrypt_office_file(encrypted, password)
        if error_code == "wrong_password":
            return fail(error_code, "The workbook password is wrong.")
        if error_code == "integrity_failed":
            return fail(error_code, "The workbook integrity check failed.")
        if error_code is not None:
            return fail(error_code, "The workbook could not be decrypted safely.")
    finally:
        password = ""
        for index in range(len(password_bytes)):
            password_bytes[index] = 0
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
