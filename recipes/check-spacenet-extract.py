#!/usr/bin/env python3
"""Authored stdlib-only tests for bounded source archive extraction."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('extractor', Path(__file__).with_name('spacenet-psrgb16-extract.py'))
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
NAME = 'SpaceNet_Roads_Sample/AOI_authored/RGB-PanSharpen/authored.tif'
DATA = b'authored source bytes; not external imagery'
SELECTED = [{'member': NAME, 'bytes': len(DATA), 'sha256': hashlib.sha256(DATA).hexdigest()}]


class ExtractionTests(unittest.TestCase):
    def archive(self, root, entries):
        path = root / 'SN3_roads_sample.tar.gz'
        with tarfile.open(path, 'w:gz') as stream:
            for name, kind, data in entries:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.size = len(data) if kind == tarfile.REGTYPE else 0
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                    member.linkname = '/escape'
                stream.addfile(member, io.BytesIO(data) if member.isreg() else None)
        return path

    def test_valid_preflight_extract_existing_refusal_and_no_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = self.archive(root, [(NAME, tarfile.REGTYPE, DATA)])
            notice = root / 'LICENSE.md'
            notice.write_text('Authored notice')
            lock = root.parent / (root.name + '.json')
            # The lock fixture is outside the source tree; cleanup is test-owned.
            try:
                lock.write_text(json.dumps({'members': SELECTED, 'assets': [
                    {'path': p.name, 'bytes': p.stat().st_size, 'sha256': r.digest(p)} for p in (archive, notice)]}))
                with patch.object(r, 'LOCK', lock):
                    self.assertEqual(r.run(root)['selected_members'], 1)
                    self.assertFalse((root / NAME).exists())
                    r.run(root, extract=True)
                    self.assertEqual((root / NAME).read_bytes(), DATA)
                    with self.assertRaisesRegex(ValueError, 'existing member'):
                        r.run(root, extract=True)
                    self.assertEqual((root / NAME).read_bytes(), DATA)
                    notice.write_text('changed')
                    with self.assertRaisesRegex(ValueError, 'notice identity'):
                        r.run(root)
            finally:
                lock.unlink(missing_ok=True)

    def test_malicious_archive_entries_and_integrity_refused(self):
        cases = [[('../escape', tarfile.REGTYPE, b'bad')],
                 [('/absolute', tarfile.REGTYPE, b'bad')],
                 [('safe/../escape', tarfile.REGTYPE, b'bad')],
                 [('link', tarfile.SYMTYPE, b'')], [('hard', tarfile.LNKTYPE, b'')],
                 [('device', tarfile.CHRTYPE, b'')], [('fifo', tarfile.FIFOTYPE, b'')],
                 [(NAME, tarfile.REGTYPE, DATA), (NAME, tarfile.REGTYPE, DATA)],
                 [(NAME, tarfile.REGTYPE, b'x' * len(DATA))],
                 [(NAME, tarfile.REGTYPE, b'short')], [(NAME, tarfile.DIRTYPE, b'')], []]
        for entries in cases:
            with self.subTest(entries=entries), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                archive = self.archive(root, entries)
                with self.assertRaises(ValueError):
                    r.inspect_archive(archive, SELECTED)
                self.assertEqual(list(root.iterdir()), [archive])

    def test_each_budget_and_duplicate_selection_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = self.archive(Path(directory), [(NAME, tarfile.REGTYPE, DATA)])
            for limit in ('MAX_MEMBERS', 'MAX_TOTAL_BYTES', 'MAX_MEMBER_BYTES', 'MAX_SELECTED_BYTES'):
                with self.subTest(limit=limit), patch.object(r, limit, 0), self.assertRaises(ValueError):
                    r.inspect_archive(archive, SELECTED)
            with self.assertRaises(ValueError):
                r.inspect_archive(archive, SELECTED + SELECTED)


if __name__ == '__main__':
    unittest.main()
