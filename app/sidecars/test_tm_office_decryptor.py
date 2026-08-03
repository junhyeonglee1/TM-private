from __future__ import annotations

import io
import hashlib
import hmac
import json
import struct
import subprocess
import sys
import unittest
import zipfile
from collections.abc import Callable
from pathlib import Path
from unittest import mock

from msoffcrypto.format.ooxml import OOXMLFile
from msoffcrypto.exceptions import DecryptionError, InvalidKeyError
from msoffcrypto.method.ecma376_agile import (
    ECMA376Agile,
    ECMA376AgileEncryptionInfo,
    _encrypt_aes_cbc,
    _normalize_key,
    blkKey_VerifierHashInput,
    blkKey_dataIntegrity1,
    blkKey_dataIntegrity2,
    blkKey_encryptedKeyValue,
    blkKey_encryptedVerifierHashValue,
)
from msoffcrypto.method.container.ecma376_encrypted import ECMA376Encrypted

import tm_office_decryptor


SCRIPT = Path(__file__).with_name("tm_office_decryptor.py")
PASSWORD = "synthetic-password"


def agile_sha1_password_info(password: str) -> dict[str, object]:
    salt = bytes(range(16))
    verifier_input = bytes(range(16, 32))
    spin_count = 7
    key_bits = 128
    iterated = ECMA376Agile._derive_iterated_hash_from_password(
        password,
        salt,
        "SHA1",
        spin_count,
    )
    input_key = ECMA376Agile._derive_encryption_key(
        iterated.digest(),
        blkKey_VerifierHashInput,
        "SHA1",
        key_bits,
    )
    hash_key = ECMA376Agile._derive_encryption_key(
        iterated.digest(),
        blkKey_encryptedVerifierHashValue,
        "SHA1",
        key_bits,
    )
    verifier_hash = hashlib.sha1(verifier_input).digest()
    return {
        "passwordHashAlgorithm": "SHA1",
        "passwordKeyBits": key_bits,
        "spinValue": spin_count,
        "passwordSalt": salt,
        "encryptedVerifierHashInput": _encrypt_aes_cbc(
            verifier_input,
            input_key,
            salt,
        ),
        "encryptedVerifierHashValue": _encrypt_aes_cbc(
            verifier_hash + (b"\0" * 12),
            hash_key,
            salt,
        ),
    }


def agile_sha1_integrity_context(
    payload: bytes = b"synthetic encrypted package",
) -> tuple[dict[str, object], bytes, bytes]:
    secret_key = bytes(range(32, 48))
    salt = bytes(range(48, 64))
    hmac_key = bytes(range(64, 84))
    first_iv = _normalize_key(
        hashlib.sha1(salt + blkKey_dataIntegrity1).digest(),
        16,
    )
    second_iv = _normalize_key(
        hashlib.sha1(salt + blkKey_dataIntegrity2).digest(),
        16,
    )
    hmac_value = hmac.new(hmac_key, payload, hashlib.sha1).digest()
    info = {
        "keyDataHashAlgorithm": "SHA1",
        "keyDataBlockSize": 16,
        "keyDataSalt": salt,
        "encryptedHmacKey": _encrypt_aes_cbc(
            hmac_key + (b"\0" * 12),
            secret_key,
            first_iv,
        ),
        "encryptedHmacValue": _encrypt_aes_cbc(
            hmac_value + (b"\0" * 12),
            secret_key,
            second_iv,
        ),
    }
    return info, secret_key, payload


def mock_agile_sha512_file() -> mock.Mock:
    office_file = mock.Mock()
    office_file.type = "agile"
    office_file.info = {
        "passwordHashAlgorithm": "SHA512",
        "keyDataHashAlgorithm": "SHA512",
    }
    return office_file


def synthetic_xlsx() -> bytes:
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(
            "[Content_Types].xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>""",
        )
        archive.writestr(
            "_rels/.rels",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>""",
        )
        archive.writestr(
            "xl/workbook.xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Synthetic" sheetId="1" r:id="rId1"/></sheets>
</workbook>""",
        )
        archive.writestr(
            "xl/_rels/workbook.xml.rels",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>""",
        )
        archive.writestr(
            "xl/worksheets/sheet1.xml",
            """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>synthetic-only</t></is></c></row></sheetData>
</worksheet>""",
        )
        # msoffcrypto's test-only writer places very small encrypted payloads in
        # an OLE mini-stream. Keep this synthetic package above the mini-stream
        # threshold so the fixture exercises the same regular stream layout as
        # exported transaction workbooks. The unreferenced entry is harmless and
        # deterministic; production code never creates or rewrites a workbook.
        archive.writestr(
            "xl/synthetic-padding.bin",
            bytes(range(256)) * 24,
            compress_type=zipfile.ZIP_STORED,
        )
    return output.getvalue()


def encrypt_xlsx(plaintext: bytes) -> bytes:
    encrypted = io.BytesIO()
    OOXMLFile(io.BytesIO(plaintext)).encrypt(PASSWORD, encrypted)
    return encrypted.getvalue()


def encrypt_xlsx_sha1(
    plaintext: bytes,
    *,
    corrupt_payload: bool = False,
    declared_cipher_algorithm: str = "AES",
    descriptor_transform: Callable[[bytes], bytes] | None = None,
    invalid_key_data_salt_length: bool = False,
    nonzero_verifier_padding: bool = False,
    nonzero_hmac_padding: bool = False,
) -> bytes:
    info = ECMA376AgileEncryptionInfo()
    for params in (info.keyData, info.encryptedKey):
        params.cipherName = declared_cipher_algorithm
        params.hashName = "SHA1"
        params.saltSize = 16
        params.blockSize = 16
        params.keyBits = 128
        params.hashSize = 20
    info.spinCount = 7
    info.encryptedKey.saltValue = bytes(range(16))
    info.keyData.saltValue = bytes(range(16, 32))

    iterated = ECMA376Agile._derive_iterated_hash_from_password(
        PASSWORD,
        info.encryptedKey.saltValue,
        info.encryptedKey.hashName,
        info.spinCount,
    )
    input_key = ECMA376Agile._derive_encryption_key(
        iterated.digest(),
        blkKey_VerifierHashInput,
        info.encryptedKey.hashName,
        info.encryptedKey.keyBits,
    )
    hash_key = ECMA376Agile._derive_encryption_key(
        iterated.digest(),
        blkKey_encryptedVerifierHashValue,
        info.encryptedKey.hashName,
        info.encryptedKey.keyBits,
    )
    secret_key_key = ECMA376Agile._derive_encryption_key(
        iterated.digest(),
        blkKey_encryptedKeyValue,
        info.encryptedKey.hashName,
        info.encryptedKey.keyBits,
    )
    verifier_input = bytes(range(32, 48))
    verifier_hash = hashlib.sha1(verifier_input).digest()
    verifier_padding = bytearray(b"\0" * 12)
    if nonzero_verifier_padding:
        verifier_padding[-1] = 1
    info.encryptedVerifierHashInput = _encrypt_aes_cbc(
        verifier_input,
        input_key,
        info.encryptedKey.saltValue,
    )
    info.encryptedVerifierHashValue = _encrypt_aes_cbc(
        verifier_hash + bytes(verifier_padding),
        hash_key,
        info.encryptedKey.saltValue,
    )
    secret_key = bytes(range(48, 64))
    info.encryptedKeyValue = _encrypt_aes_cbc(
        secret_key,
        secret_key_key,
        info.encryptedKey.saltValue,
    )

    encrypted_payload = ECMA376Agile.encrypt_payload(
        io.BytesIO(plaintext),
        info.keyData,
        secret_key,
        info.keyData.saltValue,
    )
    hmac_key = bytes(range(64, 84))
    first_iv = _normalize_key(
        hashlib.sha1(info.keyData.saltValue + blkKey_dataIntegrity1).digest(),
        info.keyData.blockSize,
    )
    second_iv = _normalize_key(
        hashlib.sha1(info.keyData.saltValue + blkKey_dataIntegrity2).digest(),
        info.keyData.blockSize,
    )
    hmac_value = hmac.new(hmac_key, encrypted_payload, hashlib.sha1).digest()
    hmac_padding = bytearray(b"\0" * 12)
    if nonzero_hmac_padding:
        hmac_padding[-1] = 1
    info.encryptedHmacKey = _encrypt_aes_cbc(
        hmac_key + (b"\0" * 12),
        secret_key,
        first_iv,
    )
    info.encryptedHmacValue = _encrypt_aes_cbc(
        hmac_value + bytes(hmac_padding),
        secret_key,
        second_iv,
    )
    if corrupt_payload:
        changed_payload = bytearray(encrypted_payload)
        changed_payload[-1] ^= 1
        encrypted_payload = bytes(changed_payload)

    if invalid_key_data_salt_length:
        info.keyData.saltValue = b"x"
    descriptor = info.toEncryptionDescriptor().encode("utf-8")
    if descriptor_transform is not None:
        descriptor = descriptor_transform(descriptor)
    encryption_info = info.getEncryptionDescriptorHeader() + descriptor
    encrypted = io.BytesIO()
    ECMA376Encrypted(encrypted_payload, encryption_info).write_to(encrypted)
    return encrypted.getvalue()


def invoke(encrypted: bytes, password: str) -> tuple[int, dict[str, object], bytes, bytes]:
    password_bytes = password.encode("utf-8")
    request = struct.pack(">I", len(password_bytes)) + password_bytes + encrypted
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        input=request,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=20,
    )
    header_bytes, separator, body = result.stdout.partition(b"\n")
    if not separator:
        raise AssertionError("sidecar response did not contain a JSON header")
    return result.returncode, json.loads(header_bytes), body, result.stderr


class OfficeDecryptorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.plaintext = synthetic_xlsx()
        cls.encrypted = encrypt_xlsx(cls.plaintext)

    def test_correct_password_round_trips_entire_workbook_in_memory(self) -> None:
        returncode, header, body, stderr = invoke(self.encrypted, PASSWORD)
        self.assertEqual(returncode, 0)
        self.assertEqual(header["ok"], True)
        self.assertEqual(header["length"], len(self.plaintext))
        self.assertEqual(body, self.plaintext)
        self.assertEqual(stderr, b"")

    def test_agile_sha1_password_accepts_required_zero_padding(self) -> None:
        info = agile_sha1_password_info(PASSWORD)
        self.assertFalse(
            ECMA376Agile.verify_password(
                PASSWORD,
                info["passwordSalt"],
                info["passwordHashAlgorithm"],
                info["encryptedVerifierHashInput"],
                info["encryptedVerifierHashValue"],
                info["spinValue"],
                info["passwordKeyBits"],
            )
        )
        self.assertTrue(tm_office_decryptor._verify_agile_sha1_password(info, PASSWORD))
        self.assertFalse(
            tm_office_decryptor._verify_agile_sha1_password(info, "wrong-password")
        )

    def test_agile_sha1_full_container_round_trips_in_subprocess(self) -> None:
        encrypted = encrypt_xlsx_sha1(self.plaintext)
        self.assertTrue(
            tm_office_decryptor._has_exact_agile_sha1_aes128_metadata(encrypted)
        )
        returncode, header, body, stderr = invoke(encrypted, PASSWORD)
        self.assertEqual(returncode, 0)
        self.assertEqual(header["ok"], True)
        self.assertEqual(body, self.plaintext)
        self.assertEqual(stderr, b"")

    def test_agile_sha1_full_container_rejects_wrong_password(self) -> None:
        returncode, header, body, stderr = invoke(
            encrypt_xlsx_sha1(self.plaintext),
            "wrong-password",
        )
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["code"], "wrong_password")
        self.assertEqual(body, b"")
        self.assertEqual(stderr, b"")

    def test_agile_sha1_full_container_rejects_changed_payload(self) -> None:
        returncode, header, body, stderr = invoke(
            encrypt_xlsx_sha1(self.plaintext, corrupt_payload=True),
            PASSWORD,
        )
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["code"], "integrity_failed")
        self.assertEqual(body, b"")
        self.assertEqual(stderr, b"")

    def test_agile_sha1_full_container_rejects_unapproved_metadata(self) -> None:
        returncode, header, body, _ = invoke(
            encrypt_xlsx_sha1(
                self.plaintext,
                declared_cipher_algorithm="UNAPPROVED",
            ),
            PASSWORD,
        )
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["code"], "unsupported_encryption")
        self.assertEqual(body, b"")

    def test_agile_sha1_rejects_doctype_before_library_xml_parser(self) -> None:
        def add_doctype(descriptor: bytes) -> bytes:
            declaration_end = descriptor.index(b"?>") + 2
            return (
                descriptor[:declaration_end]
                + b'<!DOCTYPE encryption [<!ENTITY x "blocked">]>'
                + descriptor[declaration_end:]
            )

        encrypted = encrypt_xlsx_sha1(
            self.plaintext,
            descriptor_transform=add_doctype,
        )
        with mock.patch.object(tm_office_decryptor.msoffcrypto, "OfficeFile") as parser:
            code, body = tm_office_decryptor.decrypt_office_file(encrypted, PASSWORD)
        self.assertEqual(code, "unsupported_encryption")
        self.assertEqual(body, b"")
        parser.assert_not_called()

    def test_agile_sha1_rejects_parser_differential_topology(self) -> None:
        def add_unnamespaced_key_data(descriptor: bytes) -> bytes:
            root_start = descriptor.index(b"<encryption ")
            root_end = descriptor.index(b">", root_start) + 1
            return (
                descriptor[:root_end]
                + b'<keyData xmlns="" />'
                + descriptor[root_end:]
            )

        encrypted = encrypt_xlsx_sha1(
            self.plaintext,
            descriptor_transform=add_unnamespaced_key_data,
        )
        with mock.patch.object(tm_office_decryptor.msoffcrypto, "OfficeFile") as parser:
            code, body = tm_office_decryptor.decrypt_office_file(encrypted, PASSWORD)
        self.assertEqual(code, "unsupported_encryption")
        self.assertEqual(body, b"")
        parser.assert_not_called()

    def test_agile_sha1_rejects_invalid_base64_length_before_library_parser(
        self,
    ) -> None:
        encrypted = encrypt_xlsx_sha1(
            self.plaintext,
            invalid_key_data_salt_length=True,
        )
        with mock.patch.object(tm_office_decryptor.msoffcrypto, "OfficeFile") as parser:
            code, body = tm_office_decryptor.decrypt_office_file(encrypted, PASSWORD)
        self.assertEqual(code, "unsupported_encryption")
        self.assertEqual(body, b"")
        parser.assert_not_called()

    def test_agile_sha1_rejects_non_utf8_descriptor_before_library_parser(
        self,
    ) -> None:
        def encode_utf16(descriptor: bytes) -> bytes:
            text = descriptor.decode("utf-8").replace(
                'encoding="UTF-8"',
                'encoding="UTF-16"',
            )
            return text.encode("utf-16")

        encrypted = encrypt_xlsx_sha1(
            self.plaintext,
            descriptor_transform=encode_utf16,
        )
        with mock.patch.object(tm_office_decryptor.msoffcrypto, "OfficeFile") as parser:
            code, body = tm_office_decryptor.decrypt_office_file(encrypted, PASSWORD)
        self.assertEqual(code, "unsupported_encryption")
        self.assertEqual(body, b"")
        parser.assert_not_called()

    def test_agile_sha1_full_container_rejects_nonzero_verifier_padding(
        self,
    ) -> None:
        returncode, header, body, _ = invoke(
            encrypt_xlsx_sha1(self.plaintext, nonzero_verifier_padding=True),
            PASSWORD,
        )
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["code"], "wrong_password")
        self.assertEqual(body, b"")

    def test_agile_sha1_full_container_rejects_nonzero_hmac_padding(self) -> None:
        returncode, header, body, _ = invoke(
            encrypt_xlsx_sha1(self.plaintext, nonzero_hmac_padding=True),
            PASSWORD,
        )
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["code"], "integrity_failed")
        self.assertEqual(body, b"")

    def test_agile_sha1_load_key_uses_fixed_verifier_then_derives_key(self) -> None:
        office_file = mock.Mock()
        office_file.type = "agile"
        office_file.info = agile_sha1_password_info(PASSWORD)
        tm_office_decryptor._load_key_with_verified_password(
            office_file,
            PASSWORD,
            allow_agile_sha1_compatibility=True,
        )
        office_file.load_key.assert_called_once_with(
            password=PASSWORD,
            verify_password=False,
        )

    def test_agile_sha1_wrong_password_never_derives_key(self) -> None:
        office_file = mock.Mock()
        office_file.type = "agile"
        office_file.info = agile_sha1_password_info(PASSWORD)
        with self.assertRaises(InvalidKeyError):
            tm_office_decryptor._load_key_with_verified_password(
                office_file,
                "wrong-password",
                allow_agile_sha1_compatibility=True,
            )
        office_file.load_key.assert_not_called()

    def test_agile_sha1_integrity_accepts_required_zero_padding(self) -> None:
        info, secret_key, payload = agile_sha1_integrity_context()
        office_file = mock.Mock()
        office_file.info = info
        office_file.secret_key = secret_key
        office_file.file.openstream.return_value = io.BytesIO(payload)
        self.assertTrue(tm_office_decryptor._verify_agile_sha1_integrity(office_file))

    def test_agile_sha1_integrity_rejects_changed_payload(self) -> None:
        info, secret_key, payload = agile_sha1_integrity_context()
        office_file = mock.Mock()
        office_file.info = info
        office_file.secret_key = secret_key
        office_file.file.openstream.return_value = io.BytesIO(payload + b"changed")
        self.assertFalse(tm_office_decryptor._verify_agile_sha1_integrity(office_file))

    def test_agile_sha1_changed_payload_is_never_decrypted(self) -> None:
        info, secret_key, payload = agile_sha1_integrity_context()
        office_file = mock.Mock()
        office_file.type = "agile"
        office_file.info = info
        office_file.secret_key = secret_key
        office_file.file.openstream.return_value = io.BytesIO(payload + b"changed")
        with self.assertRaises(InvalidKeyError):
            tm_office_decryptor._decrypt_with_verified_integrity(
                office_file,
                io.BytesIO(),
                allow_agile_sha1_compatibility=True,
            )
        office_file.decrypt.assert_not_called()

    def test_agile_sha1_decrypt_disables_only_broken_library_integrity_check(
        self,
    ) -> None:
        info, secret_key, payload = agile_sha1_integrity_context()
        office_file = mock.Mock()
        office_file.type = "agile"
        office_file.info = info
        office_file.secret_key = secret_key
        office_file.file.openstream.return_value = io.BytesIO(payload)
        output = io.BytesIO()
        tm_office_decryptor._decrypt_with_verified_integrity(
            office_file,
            output,
            allow_agile_sha1_compatibility=True,
        )
        office_file.decrypt.assert_called_once_with(output, verify_integrity=False)

    def test_wrong_password_is_retryable_and_never_echoes_password(self) -> None:
        wrong = "definitely-wrong"
        returncode, header, body, stderr = invoke(self.encrypted, wrong)
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["ok"], False)
        self.assertEqual(header["code"], "wrong_password")
        self.assertEqual(body, b"")
        self.assertNotIn(wrong.encode(), json.dumps(header).encode() + stderr)

    def test_integrity_failure_is_not_reported_as_wrong_password(self) -> None:
        office_file = mock_agile_sha512_file()
        office_file.load_key.return_value = None
        office_file.decrypt.side_effect = InvalidKeyError("synthetic integrity failure")
        with mock.patch.object(tm_office_decryptor.msoffcrypto, "OfficeFile", return_value=office_file):
            code, body = tm_office_decryptor.decrypt_office_file(self.encrypted, PASSWORD)
        self.assertEqual(code, "integrity_failed")
        self.assertEqual(body, b"")
        office_file.load_key.assert_called_once_with(password=PASSWORD, verify_password=True)
        office_file.decrypt.assert_called_once_with(mock.ANY, verify_integrity=True)

    def test_key_verification_failure_is_reported_as_wrong_password(self) -> None:
        office_file = mock_agile_sha512_file()
        office_file.load_key.side_effect = InvalidKeyError("synthetic password failure")
        with mock.patch.object(tm_office_decryptor.msoffcrypto, "OfficeFile", return_value=office_file):
            code, body = tm_office_decryptor.decrypt_office_file(self.encrypted, PASSWORD)
        self.assertEqual(code, "wrong_password")
        self.assertEqual(body, b"")
        office_file.load_key.assert_called_once_with(password=PASSWORD, verify_password=True)
        office_file.decrypt.assert_not_called()

    def test_decryption_failure_is_not_reported_as_wrong_password(self) -> None:
        office_file = mock_agile_sha512_file()
        office_file.load_key.return_value = None
        office_file.decrypt.side_effect = DecryptionError("synthetic decryption failure")
        with mock.patch.object(tm_office_decryptor.msoffcrypto, "OfficeFile", return_value=office_file):
            code, body = tm_office_decryptor.decrypt_office_file(self.encrypted, PASSWORD)
        self.assertEqual(code, "decrypt_failed")
        self.assertEqual(body, b"")
        office_file.load_key.assert_called_once_with(password=PASSWORD, verify_password=True)
        office_file.decrypt.assert_called_once_with(mock.ANY, verify_integrity=True)

    def test_plain_zip_is_rejected_without_echoing_payload(self) -> None:
        returncode, header, body, _ = invoke(self.plaintext, PASSWORD)
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["code"], "unsupported_encryption")
        self.assertEqual(body, b"")


if __name__ == "__main__":
    unittest.main()
