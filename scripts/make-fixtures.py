"""Rebuilds the KDBX fixtures of the Rust tests from the converter's test XML
(made-up data, password "test"):

  src-tauri/tests/fixtures/sic2kdbx.kdbx   converted by the current sic2kdbx
  src-tauri/tests/fixtures/unordered.kdbx  the same without canonical_order(),
                                           as older sic2kdbx versions wrote it

Run from the repository root: .venv\\Scripts\\python scripts/make-fixtures.py
"""

import os
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path[:0] = [ROOT, os.path.join(ROOT, "tests")]

import sic2kdbx  # noqa: E402
import test_sic2kdbx  # noqa: E402

FIXTURES = os.path.join(ROOT, "src-tauri", "tests", "fixtures")


def build(name):
    with tempfile.TemporaryDirectory() as tmp:
        xml = os.path.join(tmp, "export.xml")
        with open(xml, "w", encoding="utf-8") as f:
            f.write(test_sic2kdbx.XML)
        out = os.path.join(FIXTURES, name)
        if os.path.exists(out):
            os.remove(out)
        sic2kdbx.convert(sic2kdbx.parse_sic(xml), out, "test")


build("sic2kdbx.kdbx")
sic2kdbx.canonical_order = lambda kp: None
build("unordered.kdbx")
