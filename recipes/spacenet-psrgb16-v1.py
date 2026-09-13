#!/usr/bin/env python3
"""Offline, opt-in native SpaceNet RGB16 and source-validity preparation."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path, PurePosixPath
import platform
import subprocess

import numpy as np
from osgeo import gdal

gdal.UseExceptions()
gdal.SetConfigOption("GDAL_PAM_ENABLED", "NO")
ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "recipes/spacenet-psrgb16-v1.sources.json"
SELECTION = ROOT / "recipes/spacenet-psrgb16-v1.selection.json"
PACK = "common/spacenet-psrgb16"
AOIS = ["AOI_2_Vegas", "AOI_3_Paris", "AOI_4_Shanghai", "AOI_5_Khartoum"]
spec = importlib.util.spec_from_file_location("native_writer", ROOT / "recipes/rareplanes-calibration-v1.py")
native_writer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_writer)
sha256, json_write, child = native_writer.sha256, native_writer.json_write, native_writer.child


def safe_path(root, relative):
    path = PurePosixPath(relative)
    if not relative or path.is_absolute() or ".." in path.parts or path.as_posix() != relative or "\\" in relative:
        raise ValueError("unsafe relative path")
    target = root / relative
    if any(p.is_symlink() for p in [target, *target.parents]):
        raise ValueError("symlink in source path")
    if not target.resolve(strict=True).is_relative_to(root.resolve(strict=True)):
        raise ValueError("source escapes root")
    return target


def validate_lock(lock):
    selection = json.loads(SELECTION.read_text())
    if lock.get("schema_version") != 1 or lock["selection_sha256"] != sha256(SELECTION):
        raise ValueError("selection identity mismatch")
    members = lock["members"]
    if [{k: r[k] for k in ("member", "aoi", "rank", "role")} for r in members] != selection["members"]:
        raise ValueError("membership or role differs from frozen selection")
    if len(members) != 16 or len({r["member"] for r in members}) != 16:
        raise ValueError("expected sixteen distinct source chips")
    for aoi in AOIS:
        rows = [r for r in members if r["aoi"] == aoi]
        if [r["rank"] for r in rows] != [0, 3, 6, 9]:
            raise ValueError("incorrect lexical ranks")
        eligible = lock["eligible_members"][aoi]
        if len(eligible) != 10 or eligible != sorted(set(eligible)) or [r["member"] for r in rows] != [eligible[i] for i in (0, 3, 6, 9)]:
            raise ValueError("membership differs from lexical source selection")
        role = "reserved" if aoi == AOIS[-1] else "development"
        if any(r["role"] != role for r in rows):
            raise ValueError("incorrect geographic role")
    assets = lock["assets"]
    expected = {"SN3_roads_sample.tar.gz", "LICENSE.md", *(r["member"] for r in members)}
    if len(assets) != 18 or {a["path"] for a in assets} != expected:
        raise ValueError("incomplete source tree lock")
    by_path = {a["path"]: a for a in assets}
    if by_path["SN3_roads_sample.tar.gz"]["sha256"] != selection["archive_sha256"]:
        raise ValueError("archive identity differs")
    for record in members:
        if any(record[k] != by_path[record["member"]][k] for k in ("bytes", "sha256")):
            raise ValueError("member integrity differs")
    for record in assets:
        p = PurePosixPath(record["path"])
        if p.is_absolute() or ".." in p.parts or p.as_posix() != record["path"] or "\\" in record["path"]:
            raise ValueError("unsafe locked path")
        if type(record["bytes"]) is not int or record["bytes"] <= 0 or len(record["sha256"]) != 64:
            raise ValueError("invalid source identity")
        int(record["sha256"], 16)


def source_dataset(path):
    dataset = gdal.OpenEx(str(path), gdal.OF_RASTER | gdal.OF_READONLY, allowed_drivers=["GTiff"])
    if dataset is None or dataset.RasterCount != 3 or not dataset.GetSpatialRef():
        raise ValueError("expected georeferenced three-band GeoTIFF")
    if dataset.RasterXSize <= 0 or dataset.RasterYSize <= 0:
        raise ValueError("empty raster")
    for index, colour in enumerate((gdal.GCI_RedBand, gdal.GCI_GreenBand, gdal.GCI_BlueBand), 1):
        band = dataset.GetRasterBand(index)
        if band.DataType != gdal.GDT_UInt16 or band.GetColorInterpretation() != colour:
            raise ValueError("expected native UInt16 source RGB band order")
    return dataset


def metadata(dataset):
    bands = []
    for i in range(1, 4):
        b = dataset.GetRasterBand(i)
        bands.append({"index": i, "colour": gdal.GetColorInterpretationName(b.GetColorInterpretation()),
                      "stored_type": gdal.GetDataTypeName(b.DataType), "scale": b.GetScale(),
                      "offset": b.GetOffset(), "nodata": b.GetNoDataValue(),
                      "mask_flags": b.GetMaskFlags(), "description": b.GetDescription(),
                      "metadata": b.GetMetadata(), "image_structure": b.GetMetadata("IMAGE_STRUCTURE")})
    transform = list(dataset.GetGeoTransform())
    return {"width": dataset.RasterXSize, "height": dataset.RasterYSize, "bands": bands,
            "geotransform": transform, "crs_wkt": dataset.GetProjection(),
            "pixel_basis_vectors_crs_units": [[transform[1], transform[4]], [transform[2], transform[5]]],
            "metadata": dataset.GetMetadata(), "image_structure": dataset.GetMetadata("IMAGE_STRUCTURE"),
            "declared_sensor_precision_bits": None,
            "sensor_precision_note": "Not declared by the selected TIFF; UInt16 storage and observed values do not establish sensor precision.",
            "scale_offset_policy": "Retained as metadata, never applied to stored integer samples."}


def write_chip(source, output, mask_output, rows=128):
    dataset = source_dataset(source)
    width, height = dataset.RasterXSize, dataset.RasterYSize
    source_metadata = metadata(dataset)
    with output.open("xb") as pixels, mask_output.open("xb") as masks:
        for y in range(0, height, rows):
            count = min(rows, height - y)
            values = np.moveaxis(dataset.ReadAsArray(0, y, width, count), 0, -1)
            valid = np.empty(values.shape, dtype=np.uint8)
            for component in range(3):
                band = dataset.GetRasterBand(component + 1)
                good = band.GetMaskBand().ReadAsArray(0, y, width, count) != 0
                nodata = band.GetNoDataValue()
                if nodata is not None:
                    good &= values[:, :, component] != nodata
                valid[:, :, component] = good
            pixels.write(values.astype("<u2", copy=False).tobytes())
            masks.write(valid.tobytes())
    dataset = None
    result = verify_chip(source, output, mask_output)
    result["source_metadata"] = source_metadata
    return result


def verify_chip(source, output, mask_output):
    """Reopen source and both outputs; compare every integer and validity sample."""
    dataset = source_dataset(source)
    width, height = dataset.RasterXSize, dataset.RasterYSize
    if output.stat().st_size != width * height * 6 or mask_output.stat().st_size != width * height * 3:
        raise ValueError("incorrect prepared byte length")
    raw = np.memmap(output, dtype="<u2", mode="r", shape=(height, width, 3))
    mask = np.memmap(mask_output, dtype="u1", mode="r", shape=(height, width, 3))
    statistics = []
    for c in range(3):
        band = dataset.GetRasterBand(c + 1)
        nodata = band.GetNoDataValue()
        low, high, zero, maximum, missing, valid_count = 65535, 0, 0, 0, 0, 0
        for y in range(0, height, 113):
            count = min(113, height - y)
            expected = np.frombuffer(band.ReadRaster(0, y, width, count), dtype=np.uint16).reshape(count, width)
            mask_values = np.frombuffer(band.GetMaskBand().ReadRaster(0, y, width, count), dtype=np.uint8).reshape(count, width)
            expected_valid = mask_values != 0
            if nodata is not None:
                expected_valid = np.logical_and(expected_valid, np.not_equal(expected, nodata))
                missing += int(np.count_nonzero(expected == nodata))
            if not np.array_equal(raw[y:y + count, :, c], expected):
                raise ValueError(f"sample mismatch: band {c + 1}, row {y}")
            if not np.array_equal(mask[y:y + count, :, c], expected_valid.astype(np.uint8)):
                raise ValueError(f"validity mismatch: band {c + 1}, row {y}")
            low, high = min(low, int(expected.min())), max(high, int(expected.max()))
            zero += int(np.count_nonzero(expected == 0))
            maximum += int(np.count_nonzero(expected == 65535))
            valid_count += int(np.count_nonzero(expected_valid))
        statistics.append({"source_band": c + 1, "minimum": low, "maximum": high, "zero_count": zero,
                           "storage_maximum_count": maximum, "nodata": nodata, "nodata_count": missing,
                           "valid_count": valid_count, "invalid_count": width * height - valid_count,
                           "samples": width * height})
    del raw, mask
    return {"bytes": output.stat().st_size, "sha256": sha256(output),
            "image": {"width": width, "height": height, "components": 3, "precision": 16, "signed": False},
            "statistics": statistics,
            "validity": {"path": mask_output.name, "bytes": mask_output.stat().st_size, "sha256": sha256(mask_output),
                         "layout": "packed pixel-interleaved uint8; three bands; 0 invalid, 1 valid",
                         "policy": "Per source band: GDAL mask nonzero AND sample unequal to declared nodata; no annotation mask."},
            "verification": "All samples and per-band validity: reopened source and outputs, separate per-band ReadRaster with different block boundaries."}


def verify_sources(store, lock):
    source_root = store / "source"
    if source_root.is_symlink():
        raise ValueError("source root is a symlink")
    entries = list(source_root.rglob("*"))
    if any(p.is_symlink() or not (p.is_file() or p.is_dir()) for p in entries):
        raise ValueError("special source entry")
    if {p.relative_to(source_root).as_posix() for p in entries if p.is_file()} != {a["path"] for a in lock["assets"]}:
        raise ValueError("source tree differs from lock")
    for record in lock["assets"]:
        p = safe_path(source_root, record["path"])
        if p.stat().st_size != record["bytes"] or sha256(p) != record["sha256"]:
            raise ValueError(f"source identity mismatch: {record['path']}")
    for record in lock["members"]:
        dataset = source_dataset(source_root / record["member"])
        if metadata(dataset) != record["source_metadata"]:
            raise ValueError("source metadata differs from locked preflight")
    return source_root


def provenance():
    return {"recipe": {"path": "recipes/spacenet-psrgb16-v1.py", "sha256": sha256(__file__)},
            "shared_helpers": {"path": "recipes/rareplanes-calibration-v1.py", "sha256": sha256(native_writer.__file__)},
            "source_lock_sha256": sha256(LOCK), "selection_sha256": sha256(SELECTION),
            "catalogue_revision": subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
            "tools": {"gdal": gdal.VersionInfo("RELEASE_NAME"), "numpy": np.__version__, "python": platform.python_version()}}


def prepare(store, output_name):
    if subprocess.check_output(["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=normal"], text=True).strip():
        raise ValueError("preparation requires a clean committed catalogue candidate")
    if gdal.VersionInfo("RELEASE_NAME") != "3.13.0" or np.__version__ != "2.5.1":
        raise ValueError("recipe requires GDAL 3.13.0 and NumPy 2.5.1")
    store = store.resolve(strict=True)
    output = child(store, output_name)
    lock = json.loads(LOCK.read_text())
    validate_lock(lock)
    source = verify_sources(store, lock)
    output.mkdir()
    assets = []
    for record in lock["members"]:
        asset_id = Path(record["member"]).stem
        relative = asset_id + ".raw"
        result = write_chip(source / record["member"], output / relative, output / (asset_id + ".valid.u8"))
        result.update({"id": asset_id, "bundle_id": record["aoi"], "acquisition_group": record["aoi"],
                       "role": record["role"], "product": "RGB16", "source_kind": "supplier-pansharpened-rgb16",
                       "path": relative, "band_indices": [1, 2, 3], "source_path": record["member"],
                       "source_sha256": record["sha256"], "lineage": record["lineage"]})
        assets.append(result)
    json_write(output / "prepared.json", {"schema_version": 1, "pack_id": PACK, "version": "1",
               "recipe_id": "spacenet-psrgb16-v1", "layout": "packed pixel-interleaved little-endian unsigned; no row padding",
               "provenance": provenance(), "assets": assets, "sources": lock["assets"],
               "sample_policy": "Full original supplier-pansharpened RGB bands; preserve every stored integer including invalid samples. No crop, scale, stretch, resampling or pansharpening. Statistics include invalid samples; storage maxima do not prove sensor saturation.",
               "reserve_policy": "Khartoum permits source validation and untimed lossless exactness only; no lossy or performance measurements. Unknown per-chip parent lineage prevents an independence/holdout claim."})
    return {"prepared": str(output / "prepared.json"), "sha256": sha256(output / "prepared.json"), "assets": len(assets)}


def verify(store, prepared_name):
    store = store.resolve(strict=True)
    prepared = safe_path(store, prepared_name)
    if Path(prepared_name).name != prepared_name:
        raise ValueError("prepared must be a direct child")
    lock = json.loads(LOCK.read_text())
    validate_lock(lock)
    source = verify_sources(store, lock)
    manifest_path = safe_path(prepared, "prepared.json")
    manifest = json.loads(manifest_path.read_text())
    if manifest["pack_id"] != PACK or manifest["version"] != "1" or manifest["provenance"]["source_lock_sha256"] != sha256(LOCK):
        raise ValueError("prepared pack/lock mismatch")
    if len(manifest["assets"]) != 16:
        raise ValueError("incomplete prepared cohort")
    for asset, record in zip(manifest["assets"], lock["members"], strict=True):
        if asset["id"] != Path(record["member"]).stem or asset["role"] != record["role"] or asset["source_path"] != record["member"] or asset["source_sha256"] != record["sha256"]:
            raise ValueError("prepared source membership mismatch")
        result = verify_chip(source / record["member"], safe_path(prepared, asset["path"]), safe_path(prepared, asset["validity"]["path"]))
        for field in ("sha256", "bytes", "image", "statistics", "validity", "verification"):
            if result[field] != asset[field]:
                raise ValueError(f"prepared {field} mismatch")
        contract = {"bundle_id": record["aoi"], "acquisition_group": record["aoi"], "product": "RGB16",
                    "source_kind": "supplier-pansharpened-rgb16", "band_indices": [1, 2, 3], "lineage": record["lineage"]}
        if any(asset[k] != value for k, value in contract.items()):
            raise ValueError("prepared sample or lineage contract mismatch")
        if asset["source_metadata"] != record["source_metadata"]:
            raise ValueError("prepared metadata mismatch")
    return {"prepared_sha256": sha256(manifest_path), "assets": 16, "verification": "all source samples and validity compared"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "verify"))
    parser.add_argument("--store", required=True, type=Path)
    parser.add_argument("--output-name", default="prepared")
    args = parser.parse_args()
    print(json.dumps((prepare if args.command == "prepare" else verify)(args.store, args.output_name), sort_keys=True))


if __name__ == "__main__":
    main()
