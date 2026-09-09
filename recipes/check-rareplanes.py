#!/usr/bin/env python3
"""Offline tests using only project-authored arithmetic GeoTIFFs."""
import importlib.util
import os
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np
from osgeo import gdal, osr

spec = importlib.util.spec_from_file_location("recipe", Path(__file__).with_name("rareplanes-calibration-v1.py"))
recipe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recipe)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rareplanes-authored-", dir=os.environ.get("TMPDIR"))
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def make_tiff(self, name="source.tif", precision=16, count=8):
        path = self.root / name
        dataset = gdal.GetDriverByName("GTiff").Create(str(path), 3, 2, count,
                  gdal.GDT_Byte if precision == 8 else gdal.GDT_UInt16)
        srs = osr.SpatialReference()
        srs.SetLocalCS("project-authored test grid")
        dataset.SetSpatialRef(srs)
        dataset.SetGeoTransform((0, 1, 0, 0, 0, -1))
        samples = []
        for i in range(count):
            values = np.array([[0, 1 + i, 255], [256 + i, 10000 + i, 65535]], dtype=np.uint16)
            if precision == 8:
                values = np.array([[0, 1 + i, 255], [15, 120 + i, 254]], dtype=np.uint8)
            dataset.GetRasterBand(i + 1).WriteArray(values)
            dataset.GetRasterBand(i + 1).SetNoDataValue(0)
            samples.append(values)
        dataset = None
        return path, samples

    def test_rgb_order_unsigned_little_endian_and_full_range(self):
        source, samples = self.make_tiff()
        output = self.root / "rgb.raw"
        report = recipe.write_raster(source, output, [5, 3, 2], 16, 8, rows=1)
        expected = b"".join(struct.pack("<HHH", *(int(samples[i][y, x]) for i in [4, 2, 1]))
                            for y in range(2) for x in range(3))
        self.assertEqual(output.read_bytes(), expected)
        self.assertEqual(report["image"], {"width": 3, "height": 2, "components": 3, "precision": 16, "signed": False})
        for stats in report["statistics"]:
            self.assertEqual((stats["minimum"], stats["maximum"], stats["nodata"], stats["nodata_count"], stats["storage_maximum_count"]), (0, 65535, 0, 1, 1))

    def test_u8_preserves_values_and_pan_has_single_component(self):
        source, samples = self.make_tiff(precision=8, count=3)
        output = self.root / "rgb.raw"
        recipe.write_raster(source, output, [1, 2, 3], 8, 3)
        expected = bytes(int(samples[c][y, x]) for y in range(2) for x in range(3) for c in range(3))
        self.assertEqual(output.read_bytes(), expected)
        source, samples = self.make_tiff("pan.tif", count=1)
        output = self.root / "pan.raw"
        recipe.write_raster(source, output, [1], 16, 1)
        self.assertEqual(output.read_bytes(), b"".join(struct.pack("<H", int(x)) for x in samples[0].flat))

    def test_wrong_type_and_non_tiff_fail(self):
        source, _ = self.make_tiff(precision=8)
        with self.assertRaises(ValueError):
            recipe.write_raster(source, self.root / "bad.raw", [1], 16, 8)
        path = self.root / "text.tif"
        path.write_text("not a TIFF")
        with self.assertRaises((ValueError, RuntimeError)):
            recipe.write_raster(path, self.root / "bad.raw", [1], 16, 1)
        self.assertFalse((self.root / "bad.raw").exists())

    def test_non_identity_scale_rejected(self):
        source, _ = self.make_tiff()
        dataset = gdal.Open(str(source), gdal.GA_Update)
        dataset.GetRasterBand(1).SetScale(0.1)
        dataset = None
        with self.assertRaises(ValueError):
            recipe.write_raster(source, self.root / "bad.raw", [1], 16, 8)

    def test_overwrite_and_escape_refused(self):
        source, _ = self.make_tiff()
        output = self.root / "keep.raw"
        output.write_bytes(b"keep")
        with self.assertRaises(FileExistsError):
            recipe.write_raster(source, output, [1], 16, 8)
        self.assertEqual(output.read_bytes(), b"keep")
        for name in ["keep.raw", "..", "../outside", "/absolute"]:
            with self.assertRaises(ValueError):
                recipe.child(self.root, name)
        (self.root / "link").symlink_to(self.root / "missing")
        with self.assertRaises(ValueError):
            recipe.child(self.root, "link")

    def test_planar_order_and_endianness(self):
        source, _ = self.make_tiff()
        interleaved, planar = self.root / "rgb.raw", self.root / "planar.raw"
        result = recipe.write_raster(source, interleaved, [5, 3, 2], 16, 8)
        recipe.write_planar(interleaved, planar, result["image"])
        words = struct.unpack("<" + "H" * 18, interleaved.read_bytes())
        self.assertEqual(planar.read_bytes(), b"".join(struct.pack("<H", words[p * 3 + c]) for c in range(3) for p in range(6)))
        with self.assertRaises(FileExistsError):
            recipe.write_planar(interleaved, planar, result["image"])
        wrong = dict(result["image"], height=3)
        with self.assertRaises(ValueError):
            recipe.write_planar(interleaved, self.root / "wrong.raw", wrong)


if __name__ == "__main__":
    unittest.main()
