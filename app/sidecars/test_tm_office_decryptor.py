from __future__ import annotations

import io
import json
import struct
import subprocess
import sys
import unittest
import zipfile
from pathlib import Path

from msoffcrypto.format.ooxml import OOXMLFile


SCRIPT = Path(__file__).with_name("tm_office_decryptor.py")
PASSWORD = "synthetic-password"


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

    def test_wrong_password_is_retryable_and_never_echoes_password(self) -> None:
        wrong = "definitely-wrong"
        returncode, header, body, stderr = invoke(self.encrypted, wrong)
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["ok"], False)
        self.assertEqual(header["code"], "wrong_password_or_corrupt")
        self.assertEqual(body, b"")
        self.assertNotIn(wrong.encode(), json.dumps(header).encode() + stderr)

    def test_plain_zip_is_rejected_without_echoing_payload(self) -> None:
        returncode, header, body, _ = invoke(self.plaintext, PASSWORD)
        self.assertNotEqual(returncode, 0)
        self.assertEqual(header["code"], "unsupported_encryption")
        self.assertEqual(body, b"")


if __name__ == "__main__":
    unittest.main()
