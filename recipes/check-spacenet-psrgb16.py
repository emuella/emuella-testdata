#!/usr/bin/env python3
"""Offline contract checks; authored raster checks use optional GDAL/NumPy."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "recipes/spacenet-psrgb16-v1.sources.json"
HAS_RASTER = importlib.util.find_spec("osgeo") is not None and importlib.util.find_spec("numpy") is not None


class ContractTests(unittest.TestCase):
    def test_integrity_and_frozen_selection(self):
        lock = json.loads(LOCK.read_text())
        selected_path = ROOT / "recipes/spacenet-psrgb16-v1.selection.json"
        selected = json.loads(selected_path.read_text())
        self.assertEqual(hashlib.sha256(selected_path.read_bytes()).hexdigest(), lock['selection_sha256'])
        self.assertEqual([{k: a[k] for k in ('member', 'aoi', 'rank', 'role')} for a in lock['members']], selected['members'])
        inventory = tomllib.loads((ROOT / "inventories/common/spacenet-psrgb16-v1.toml").read_text())
        manifest = tomllib.loads((ROOT / "manifests/common/spacenet-psrgb16.toml").read_text())
        self.assertEqual([{k: a[k] for k in ('path', 'bytes', 'sha256')} for a in inventory['assets']], lock['assets'])
        encoded = ''.join(f"{a['sha256']}\t{a['bytes']}\t{a['path']}\n" for a in sorted(lock['assets'], key=lambda a: a['path']))
        self.assertEqual(hashlib.sha256(encoded.encode()).hexdigest(), manifest['materialization']['expected_tree_sha256'])
        self.assertEqual(manifest['source']['archive_sha256'], selected['archive_sha256'])
        self.assertEqual(manifest['review_state'], 'locked')
        self.assertEqual(len(lock['members']), 16)
        for aoi in ('AOI_2_Vegas', 'AOI_3_Paris', 'AOI_4_Shanghai', 'AOI_5_Khartoum'):
            members = [a for a in lock['members'] if a['aoi'] == aoi]
            self.assertEqual([a['rank'] for a in members], [0, 3, 6, 9])
            self.assertEqual({a['role'] for a in members}, {'reserved' if 'Khartoum' in aoi else 'development'})
            eligible = lock['eligible_members'][aoi]
            self.assertEqual(len(eligible), 10)
            self.assertEqual(eligible, sorted(eligible))
            self.assertEqual([a['member'] for a in members], [eligible[i] for i in (0, 3, 6, 9)])
        for a in lock['members']:
            self.assertEqual([b['colour'] for b in a['source_metadata']['bands']], ['Red', 'Green', 'Blue'])
            self.assertIsNone(a['source_metadata']['declared_sensor_precision_bits'])
            self.assertFalse(a['lineage']['holdout_claim'])
        for suite_path in (ROOT / 'suites').rglob('*.toml'):
            self.assertNotIn('common/spacenet-psrgb16', suite_path.read_text())


@unittest.skipUnless(HAS_RASTER, 'authored raster probes require optional GDAL and NumPy')
class RasterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('spacenet_recipe', ROOT / 'recipes/spacenet-psrgb16-v1.py')
        cls.recipe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.recipe)

    def create_source(self, root, nodata=False, mask=False, wrong_type=False):
        r = self.recipe
        gdal, np = r.gdal, r.np
        path = root / 'authored.tif'
        # WKT avoids relying on an installed PROJ database.
        dataset = gdal.GetDriverByName('GTiff').Create(str(path), 7, 229, 3, gdal.GDT_Byte if wrong_type else gdal.GDT_UInt16)
        dataset.SetProjection('LOCAL_CS["Authored",LOCAL_DATUM["Authored",0],UNIT["metre",1]]')
        dataset.SetGeoTransform([12, 0.31, 0, 42, 0, -0.31])
        values = (np.arange(7 * 229 * 3, dtype=np.uint16).reshape(229, 7, 3) * 13)
        values[0, 0, :] = [0, 256, 65535]
        for c, colour in enumerate((gdal.GCI_RedBand, gdal.GCI_GreenBand, gdal.GCI_BlueBand)):
            band = dataset.GetRasterBand(c + 1)
            band.SetColorInterpretation(colour)
            band.SetScale(0.5 + c)
            band.SetOffset(-11 + c)
            if nodata:
                band.SetNoDataValue(0)
            band.WriteArray(values[:, :, c])
        if mask:
            dataset.CreateMaskBand(gdal.GMF_PER_DATASET)
            validity = np.full((229, 7), 255, dtype=np.uint8)
            validity[112:115, 2] = 0
            dataset.GetRasterBand(1).GetMaskBand().WriteArray(validity)
        dataset = None
        return path, values

    def test_native_samples_masks_nodata_scale_and_independent_reopen(self):
        r = self.recipe
        for nodata, mask in ((False, False), (True, False), (False, True), (True, True)):
            with self.subTest(nodata=nodata, mask=mask), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source, values = self.create_source(root, nodata, mask)
                raw, valid = root / 'sample.raw', root / 'valid.u8'
                result = r.write_chip(source, raw, valid, rows=97)
                self.assertEqual(raw.read_bytes(), values.astype('<u2').tobytes())
                expected = r.np.ones(values.shape, dtype=r.np.uint8)
                if nodata:
                    expected[values == 0] = 0
                if mask:
                    expected[112:115, 2, :] = 0
                self.assertEqual(valid.read_bytes(), expected.tobytes())
                self.assertEqual(result['source_metadata']['bands'][0]['scale'], 0.5)
                self.assertEqual(result['source_metadata']['bands'][0]['offset'], -11)
                self.assertEqual(result['statistics'][2]['storage_maximum_count'], 1)
                with self.assertRaises(FileExistsError):
                    r.write_chip(source, raw, valid)
                with raw.open('r+b') as stream:
                    stream.write(b'\x01')
                with self.assertRaisesRegex(ValueError, 'sample mismatch'):
                    r.verify_chip(source, raw, valid)
                raw.write_bytes(values.astype('<u2').tobytes())
                with valid.open('r+b') as stream:
                    stream.write(b'\x02')
                with self.assertRaisesRegex(ValueError, 'validity mismatch'):
                    r.verify_chip(source, raw, valid)

    def test_non_native_source_and_unsafe_outputs_refused(self):
        r = self.recipe
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, _ = self.create_source(root, wrong_type=True)
            with self.assertRaises(ValueError):
                r.source_dataset(source)
            for name in ('../escape', '/absolute', '.', '..', ''):
                with self.assertRaises(ValueError):
                    r.child(root, name)
            link = root / 'link'
            link.symlink_to(source)
            with self.assertRaises(ValueError):
                r.safe_path(root, 'link')

    def test_lock_mutations_refused(self):
        r = self.recipe
        lock = json.loads(LOCK.read_text())
        r.validate_lock(lock)
        for mutate in (lambda a: a['members'][0].update(role='reserved'),
                       lambda a: a['members'].pop(),
                       lambda a: a['assets'][0].update(path='../escape'),
                       lambda a: a['assets'][-1].update(sha256='0' * 64),
                       lambda a: a.update(selection_sha256='0' * 64)):
            changed = copy.deepcopy(lock)
            mutate(changed)
            with self.assertRaises(ValueError):
                r.validate_lock(changed)


if __name__ == '__main__':
    unittest.main()
