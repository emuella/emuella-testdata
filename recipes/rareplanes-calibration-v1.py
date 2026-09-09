#!/usr/bin/env python3
"""Local-only RarePlanes preparation. No acquisition, resampling or pixel scaling."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys

import numpy as np
from osgeo import gdal

gdal.UseExceptions()
gdal.SetConfigOption("GDAL_PAM_ENABLED", "NO")
ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "recipes/rareplanes-calibration-v1.sources.json"
PRODUCTS = {"PAN16": ("PAN", [1], 16, 1), "RGB8": ("PS-RGB", [1, 2, 3], 8, 3),
            "MS16": ("MS", list(range(1, 9)), 16, 8), "RGB16": ("MS", [5, 3, 2], 16, 8)}


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def json_write(path, value):
    with Path(path).open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def child(root, name):
    """Only a new direct child can be an output; never traverse or follow links."""
    if not name or Path(name).name != name or name in (".", ".."):
        raise ValueError("output name must be one direct child")
    path = root / name
    if path.exists() or path.is_symlink():
        raise ValueError(f"refusing existing output: {path}")
    return path


def source_dataset(path, count, precision):
    dataset = gdal.OpenEx(str(path), gdal.OF_RASTER | gdal.OF_READONLY,
                          allowed_drivers=["GTiff"])
    if dataset is None or dataset.GetDriver().ShortName != "GTiff":
        raise ValueError("source must be a GeoTIFF")
    if dataset.RasterCount != count or not dataset.GetSpatialRef():
        raise ValueError("wrong band count or missing GeoTIFF spatial reference")
    expected_type = gdal.GDT_Byte if precision == 8 else gdal.GDT_UInt16
    for index in range(1, count + 1):
        band = dataset.GetRasterBand(index)
        if band.DataType != expected_type:
            raise ValueError("source must have the declared unsigned sample type")
        if band.GetScale() not in (None, 1) or band.GetOffset() not in (None, 0):
            raise ValueError("non-identity scale/offset requires separate review")
    return dataset


def write_raster(source, output, band_indices, precision, source_count, rows=128):
    """Write dataset RasterIO then verify every value through separate band reads."""
    dataset = source_dataset(source, source_count, precision)
    width, height = dataset.RasterXSize, dataset.RasterYSize
    dtype = np.dtype("u1" if precision == 8 else "<u2")
    with Path(output).open("xb") as stream:
        for y in range(0, height, rows):
            block = dataset.ReadAsArray(0, y, width, min(rows, height - y),
                                        band_list=band_indices)
            if block.ndim == 2:
                block = block[np.newaxis, ...]
            stream.write(np.moveaxis(block, 0, -1).astype(dtype, copy=False).tobytes())
    del dataset
    # Reopen both products and change API, band traversal and block boundaries.
    dataset = source_dataset(source, source_count, precision)
    raw = np.memmap(output, dtype=dtype, mode="r", shape=(height, width, len(band_indices)))
    statistics = []
    for component, index in enumerate(band_indices):
        band = dataset.GetRasterBand(index)
        nodata = band.GetNoDataValue()
        low, high, zero, maximum, missing = 2 ** precision - 1, 0, 0, 0, 0
        for y in range(0, height, 113):
            nrows = min(113, height - y)
            # ReadRaster uses native-endian bytes; the output memmap is explicitly LE.
            expected = np.frombuffer(band.ReadRaster(0, y, width, nrows),
                                     dtype=np.uint8 if precision == 8 else np.uint16).reshape(nrows, width)
            actual = raw[y:y + nrows, :, component]
            if not np.array_equal(expected, actual):
                raise ValueError(f"sample mismatch in band {index}, row {y}")
            low, high = min(low, int(expected.min())), max(high, int(expected.max()))
            zero += int(np.count_nonzero(expected == 0))
            maximum += int(np.count_nonzero(expected == 2 ** precision - 1))
            if nodata is not None:
                missing += int(np.count_nonzero(expected == nodata))
        statistics.append({"source_band": index, "minimum": low, "maximum": high,
                           "zero_count": zero, "storage_maximum_count": maximum,
                           "nodata": nodata, "nodata_count": missing,
                           "samples": width * height})
    del raw
    return {"bytes": Path(output).stat().st_size, "sha256": sha256(output),
            "image": {"width": width, "height": height, "components": len(band_indices),
                      "precision": precision, "signed": False},
            "statistics": statistics, "verification": "all samples: reopened per-band ReadRaster"}


def provenance():
    return {"recipe": {"path": "recipes/rareplanes-calibration-v1.py", "sha256": sha256(__file__)},
            "source_lock": {"path": "recipes/rareplanes-calibration-v1.sources.json", "sha256": sha256(LOCK)},
            "catalogue_revision": subprocess.check_output(
                ["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
            "tools": {"gdal": gdal.VersionInfo("RELEASE_NAME"), "numpy": np.__version__,
                      "python": platform.python_version()}}


def prepare(store, output_name):
    store = store.resolve(strict=True)
    source_root = store / "source"
    if source_root.is_symlink():
        raise ValueError("source root must not be a symlink")
    output = child(store, output_name)
    lock = json.loads(LOCK.read_text())
    if gdal.VersionInfo("RELEASE_NAME") != "3.13.0" or np.__version__ != "2.5.1":
        raise ValueError("recipe requires GDAL 3.13.0 and NumPy 2.5.1")
    expected_paths = {record["path"] for record in lock["assets"]}
    entries = list(source_root.rglob("*"))
    if any(p.is_symlink() or not (p.is_file() or p.is_dir()) for p in entries):
        raise ValueError("special source entry")
    if {p.relative_to(source_root).as_posix() for p in entries if p.is_file()} != expected_paths:
        raise ValueError("source tree differs from locked selection")
    sources = {}
    for record in lock["assets"]:
        path = source_root / record["path"]
        if path.stat().st_size != record["bytes"] or sha256(path) != record["sha256"]:
            raise ValueError(f"source lock mismatch: {record['path']}")
        sources[record["path"]] = record
    # Preflight every source before creating any derivative.
    for bundle in lock["bundles"]:
        for product, (folder, indices, precision, count) in PRODUCTS.items():
            path = source_root / f"real/train/{folder}/{bundle['id']}.tif"
            dataset = source_dataset(path, count, precision)
            dimensions = bundle["ms_dimensions" if folder == "MS" else "pan_dimensions"]
            if [dataset.RasterXSize, dataset.RasterYSize] != dimensions:
                raise ValueError("source dimensions differ from lock")
            if product == "RGB8" and [dataset.GetRasterBand(i).GetColorInterpretation()
                                      for i in indices] != [gdal.GCI_RedBand, gdal.GCI_GreenBand, gdal.GCI_BlueBand]:
                raise ValueError("PS-RGB colour interpretations differ")
    output.mkdir()
    assets = []
    for bundle in lock["bundles"]:
        (output / bundle["id"]).mkdir()
        for product, (folder, indices, precision, count) in PRODUCTS.items():
            source_path = f"real/train/{folder}/{bundle['id']}.tif"
            relative = f"{bundle['id']}/{product}.raw"
            result = write_raster(source_root / source_path, output / relative, indices, precision, count)
            result.update({"id": f"{bundle['id']}-{product}", "bundle_id": bundle["id"],
                           "product": product, "path": relative, "band_indices": indices,
                           "source_path": source_path, "source_sha256": sources[source_path]["sha256"]})
            assets.append(result)
    manifest = {"schema_version": 1, "pack_id": "common/rareplanes-calibration", "version": "1",
                "layout": "packed pixel-interleaved little-endian unsigned; no row padding",
                "provenance": provenance(), "sources": lock["assets"], "assets": assets,
                "band_evidence": lock["band_evidence"],
                "sample_policy": "Preserve full dimensions and integer values including nodata; no masking, scaling, stretch, resampling or pansharpening. Statistics include nodata. Storage maximum counts are not a claim of sensor saturation."}
    json_write(output / "prepared.json", manifest)
    return {"prepared": str(output / "prepared.json"), "sha256": sha256(output / "prepared.json"), "assets": len(assets)}


def write_planar(source, output, image):
    """Bounded row copies with a separate full sample comparison of each plane."""
    dtype = np.dtype("u1" if image["precision"] == 8 else "<u2")
    w, h, c = image["width"], image["height"], image["components"]
    if Path(source).stat().st_size != w * h * c * dtype.itemsize:
        raise ValueError("incorrect interleaved byte length")
    raw = np.memmap(source, dtype=dtype, mode="r", shape=(h, w, c))
    with Path(output).open("xb") as stream:
        for component in range(c):
            for y in range(0, h, 128):
                stream.write(raw[y:y + 128, :, component].tobytes())
    planar = np.memmap(output, dtype=dtype, mode="r", shape=(c, h, w))
    for y in range(0, h, 113):
        if not np.array_equal(np.moveaxis(planar[:, y:y + 113, :], 0, -1), raw[y:y + 113]):
            raise ValueError("planar sample mismatch")
    del raw, planar
    return {"sha256": sha256(output), "bytes": Path(output).stat().st_size}


def planar(store, prepared_name, asset_id, output_name):
    store = store.resolve(strict=True)
    output = child(store, output_name)
    if Path(prepared_name).name != prepared_name or prepared_name in (".", ".."):
        raise ValueError("invalid prepared name")
    prepared = store / prepared_name
    if prepared.is_symlink():
        raise ValueError("prepared root must not be a symlink")
    manifest_path = prepared / "prepared.json"
    manifest = json.loads(manifest_path.read_text())
    asset = next(a for a in manifest["assets"] if a["id"] == asset_id)
    source = prepared / asset["path"]
    if source.is_symlink() or not source.resolve(strict=True).is_relative_to(prepared.resolve(strict=True)):
        raise ValueError("source escapes prepared root")
    if sha256(source) != asset["sha256"]:
        raise ValueError("prepared digest mismatch")
    result = write_planar(source, output, asset["image"])
    result.update({"schema_version": 1, "path": output.name, "source_sha256": asset["sha256"], "asset_id": asset_id,
                   "prepared_sha256": sha256(manifest_path), "provenance": provenance(),
                   "layout": "packed component-planar little-endian unsigned"})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "planar"):
        command = commands.add_parser(name)
        command.add_argument("--store", required=True, type=Path)
        command.add_argument("--output-name", default="prepared" if name == "prepare" else None, required=name == "planar")
        if name == "planar":
            command.add_argument("--prepared-name", default="prepared")
            command.add_argument("--asset-id", required=True)
    args = parser.parse_args()
    if args.command == "prepare":
        result = prepare(args.store, args.output_name)
    else:
        result = planar(args.store, args.prepared_name, args.asset_id, args.output_name)
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
