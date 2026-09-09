#!/usr/bin/env python3
"""Local-only expanded RarePlanes preparation; complete unchanged sample arrays."""
import argparse
import importlib.util
import json
from pathlib import Path
import platform
import subprocess

import numpy as np
from osgeo import gdal

gdal.UseExceptions()
gdal.SetConfigOption("GDAL_PAM_ENABLED", "NO")
ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "recipes/rareplanes-expanded-v1.sources.json"
SELECTION = ROOT / "recipes/rareplanes-expanded-v1.selection.json"
PRODUCTS = {"PAN16": ("PAN", [1], 16, 1), "RGB8": ("PS-RGB", [1, 2, 3], 8, 3),
            "MS16": ("MS", list(range(1, 9)), 16, 8), "RGB16": ("MS", [5, 3, 2], 16, 8)}


spec = importlib.util.spec_from_file_location("calibration", ROOT / "recipes/rareplanes-calibration-v1.py")
calibration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(calibration)
sha256 = calibration.sha256
json_write = calibration.json_write
child = calibration.child
source_dataset = calibration.source_dataset
write_raster = calibration.write_raster
write_planar = calibration.write_planar
selection_spec = importlib.util.spec_from_file_location("selection_recipe", ROOT / "recipes/rareplanes-expanded-selection-v1.py")
selection_recipe = importlib.util.module_from_spec(selection_spec)
selection_spec.loader.exec_module(selection_recipe)


def validate_lock(lock):
    """Admit exactly the frozen full-product selection and reviewed sensor mapping."""
    selection = json.loads(SELECTION.read_text())
    if lock.get("schema_version") != 1 or sha256(SELECTION) != lock["selection"]["sha256"]:
        raise ValueError("selection identity mismatch")
    bundles = lock["bundles"]
    if len(bundles) != 12 or len({b["id"] for b in bundles}) != 12:
        raise ValueError("expected twelve distinct acquisitions")
    if len({b["metadata"]["loc_id"] for b in bundles}) != 12:
        raise ValueError("expected twelve distinct locations")
    if [{k: b[k] for k in ("id", "split", "location", "metadata")} for b in bundles] != selection["bundles"]:
        raise ValueError("bundle membership differs from frozen selection")
    expected = {"LICENSE.txt", "real/metadata_annotations/RarePlanes_Public_Metadata.csv"}
    for bundle in bundles:
        if bundle["split"] not in ("train", "test") or bundle["metadata"]["sensor"] != "WV03":
            raise ValueError("unreviewed split or sensor")
        for field in ("pan_dimensions", "ms_dimensions"):
            if len(bundle[field]) != 2 or any(type(v) is not int or v <= 0 for v in bundle[field]):
                raise ValueError("invalid full-source dimensions")
        expected.update(f"real/{bundle['split']}/{folder}/{bundle['id']}.tif" for folder in ("PAN", "MS", "PS-RGB"))
    if len(lock["assets"]) != 38 or {a["path"] for a in lock["assets"]} != expected:
        raise ValueError("source membership differs from complete selection")
    if lock["band_evidence"]["rgb16_indices"] != [5, 3, 2]:
        raise ValueError("RGB16 band mapping differs from reviewed recipe")


def provenance():
    return {"recipe": {"path": "recipes/rareplanes-expanded-v1.py", "sha256": sha256(__file__)},
            "selection_recipe": {"path": "recipes/rareplanes-expanded-selection-v1.py", "sha256": sha256(selection_recipe.__file__)},
            "sample_writer": {"path": "recipes/rareplanes-calibration-v1.py", "sha256": sha256(calibration.__file__)},
            "selection": {"path": "recipes/rareplanes-expanded-v1.selection.json", "sha256": sha256(SELECTION)},
            "source_lock": {"path": "recipes/rareplanes-expanded-v1.sources.json", "sha256": sha256(LOCK)},
            "catalogue_revision": subprocess.check_output(
                ["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
            "tools": {"gdal": gdal.VersionInfo("RELEASE_NAME"), "numpy": np.__version__,
                      "python": platform.python_version()}}


def prepare(store, output_name):
    if subprocess.check_output(["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=normal"], text=True).strip():
        raise ValueError("preparation requires a clean committed catalogue candidate")
    store = store.resolve(strict=True)
    source_root = store / "source"
    if source_root.is_symlink():
        raise ValueError("source root must not be a symlink")
    output = child(store, output_name)
    lock = json.loads(LOCK.read_text())
    validate_lock(lock)
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
    reproduced = selection_recipe.select((source_root / "real/metadata_annotations/RarePlanes_Public_Metadata.csv").read_bytes())
    if reproduced != json.loads(SELECTION.read_text()):
        raise ValueError("selection does not reproduce from locked metadata")
    # Preflight every source before creating any derivative.
    for bundle in lock["bundles"]:
        for product, (folder, indices, precision, count) in PRODUCTS.items():
            path = source_root / f"real/{bundle['split']}/{folder}/{bundle['id']}.tif"
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
            source_path = f"real/{bundle['split']}/{folder}/{bundle['id']}.tif"
            relative = f"{bundle['id']}/{product}.raw"
            result = write_raster(source_root / source_path, output / relative, indices, precision, count)
            result.update({"id": f"{bundle['id']}-{product}", "bundle_id": bundle["id"],
                           "product": product, "path": relative, "band_indices": indices,
                           "source_path": source_path, "source_sha256": sources[source_path]["sha256"]})
            assets.append(result)
    manifest = {"schema_version": 1, "pack_id": "common/rareplanes-expanded", "version": "1",
                "layout": "packed pixel-interleaved little-endian unsigned; no row padding",
                "provenance": provenance(), "sources": lock["assets"], "assets": assets,
                "band_evidence": lock["band_evidence"],
                "sample_policy": "Preserve full dimensions and integer values including nodata; no masking, scaling, stretch, resampling or pansharpening. Statistics include nodata. Storage maximum counts are not a claim of sensor saturation."}
    json_write(output / "prepared.json", manifest)
    return {"prepared": str(output / "prepared.json"), "sha256": sha256(output / "prepared.json"), "assets": len(assets)}


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
