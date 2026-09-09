# RarePlanes two-location preparation, version 1

This local calibration preserves two complete real WorldView-3 acquisitions:
`30_104001002394E000` (Aeroporto di Magadino, Switzerland) and
`47_104001001D2C7A00` (Apple Valley Airport, California). They were selected
before codec timing for contrasting geography, illumination, season and size.
They are not a representative sample or a 12-location selection. The unchanged
CSV describes both as clear skies; its cloud percentages cover the whole strip,
not the delivered airfield. Select by `image_id`; some `cat_id` cells are
spreadsheet-corrupted scientific notation.

## Rights and acquisition

The [upstream bucket notice](https://rareplanes-public.s3.us-west-2.amazonaws.com/LICENSE.txt)
applies CC BY-SA 4.0 to the RarePlanes dataset. Attribute J. Shermeyer,
T. Hossler, A. Van Etten, D. Hogan, R. Lewis and D. Kim; In-Q-Tel - CosmiQ Works
and AI.Reverie; RarePlanes Dataset, June 2020. Preserve that complete notice,
attribution and source lineage with materialised imagery and derivatives.
Catalogue code and documentation remain Apache-2.0. No image payload belongs in
Git, software packages or benchmark publications from this calibration.

Acquisition was explicitly selected and authorised on 9 September 2026 for
local preparation and benchmarking only. The eight unchanged upstream objects
are the bucket notice, metadata CSV and each selected image's PAN, PS-RGB and MS
TIFF. Their exact HTTPS URLs, retrieval date, sizes, SHA-256 digests and ETags are
in [the source lock](rareplanes-calibration-v1.sources.json). The recipe does
not download, accept terms, delete, overwrite or publish material. Acquisition
and any later publication remain separate operations.

The [catalogue manifest](../manifests/common/rareplanes-calibration.toml) stays
`reviewed` and outside qualification suites. Every object and the complete
source tree are nevertheless exactly inventoried and verified. Existing
`locked` external-pack validation requires an upstream archive; this dataset
selection consists of individual objects. If a larger pack proceeds, establish
formal direct-object locked admission separately without inventing an archive
identity or weakening existing archive validation.

## Sample and band contract

| Product | Upstream folder | One-based source bands | Storage | Resolution |
|---|---|---|---|---|
| PAN16 | PAN | 1 | unsigned 16-bit | Complete PAN |
| RGB8 | PS-RGB | 1, 2, 3 | unsigned 8-bit | Complete supplied PS-RGB |
| MS16 | MS | 1 through 8 | unsigned 16-bit | Native MS |
| RGB16 | MS | 5, 3, 2 | unsigned 16-bit | Native MS |

PS-RGB TIFFs explicitly identify Red, Green and Blue. The selected metadata CSV
identifies both sensors as `WV03`; MS TIFFs have eight U16 bands with empty
spectral descriptions, and colour interpretation Gray followed by Undefined.
Thus the TIFF tags alone do not establish RGB16 band names. The mapping uses
the documented WorldView-3 VNIR order in the
[ArcGIS WorldView-3 spectral-range table](https://pro.arcgis.com/en/pro-app/3.5/help/data/imagery/satellite-sensor-raster-types.htm)
(coastal, blue, green, yellow, red, red edge, NIR1, NIR2), consistent with
[Maxar's WorldView-3 radiometry note, Table 1](https://csda-maxar-pdfs.s3.amazonaws.com/Radiometric_Use_of_WorldView-3_v1.pdf).
Applying that sensor order to these eight-band WV03 files is an explicit
inference from the source sensor identity and documented product order, not a
claim of self-describing MS tags. The exact selected source digests bind its
scope. Another sensor or reordered source needs a new reviewed recipe.

PAN/MS are delivered processed U16 products. Although the sensor acquires VNIR
at 11 bits, this preparation preserves the supplied stored integers and declares
16-bit precision; it does not claim native sensor DNs. The
[publisher's dataset paper](https://openaccess.thecvf.com/content/WACV2021/papers/Shermeyer_RarePlanes_Synthetic_Data_Takes_Flight_WACV_2021_paper.pdf)
describes the delivered products. RGB8 already contains the upstream
pansharpening and dynamic-range processing. The recipe adds no stretch, scale,
colour conversion, pansharpening, resampling, crop or range reduction.

All source bands declare nodata zero; preserve it unchanged and include nodata
in statistics and codec comparison. No mask or replacement is applied. The
manifest reports each band's observed minimum, maximum, zero count, nodata
value/count and count at the storage maximum (255 or 65535). A storage-maximum
count is not a claim of sensor saturation; neither a reflectance ceiling nor a
sensor clipping threshold is inferred. Non-identity GDAL scale/offset metadata
is rejected for separate review. Georeferencing stays in the unchanged TIFF;
raw derivatives intentionally carry only sample geometry and lineage.

## Running locally

Requires Python with GDAL **3.13.0** bindings and NumPy **2.5.1**. The source
checkpoint used Python **3.14.4**. No network is used. Set `RAREPLANES_STORE` to
the explicitly approved persistent materialisation directory containing
`source/`; place all source and derived imagery below that directory. Disposable
builds and project-authored tests may use a separate scratch directory.

```sh
cargo run -p emuella-corpus -- verify common/rareplanes-calibration \
  --root "$RAREPLANES_STORE/source"
python3 recipes/check-rareplanes.py
python3 recipes/rareplanes-calibration-v1.py prepare \
  --store "$RAREPLANES_STORE" --output-name prepared
```

The recipe checks exact tree membership and each source digest, GeoTIFF format,
unsigned types, band counts and full dimensions before creating output. The
output must be a new direct child of the store, with no symlink or traversal.
An existing output is always refused. An interrupted output stays in place;
choose another new output name after diagnosis. Do not remove materialised
sources or derivatives as routine scratch cleanup.

Eight files use packed pixel-interleaved, unsigned little-endian samples,
without headers or row padding. Each `prepared.json` asset records
`id`, `bundle_id`, `product`, relative `path`, `sha256`, `bytes`,
`image: {width, height, components, precision, signed: false}`, source path and
digest, one-based `band_indices`, statistics and verification method. Root
`schema_version` is 1; root provenance records catalogue revision, recipe and
source-lock SHA-256, and exact tool versions. All eight products are emitted
regardless of downstream codec component limits.

The writer uses bounded dataset-level `ReadAsArray` blocks. Verification reopens
the source and output, reads each source band with `ReadRaster` in different
block boundaries and compares every value to the little-endian output mapping.
It therefore checks band order, byte order and complete image coverage rather
than merely hashing the buffer used to write. Statistics come from those
independent source reads.

For tools requiring component-planar input, derive a new local file:

```sh
python3 recipes/rareplanes-calibration-v1.py planar \
  --store "$RAREPLANES_STORE" --prepared-name prepared \
  --asset-id 30_104001002394E000-RGB16 \
  --output-name calibration-planar-30-RGB16.raw
```

This command checks the interleaved digest and length, copies unchanged
little-endian sample words into complete component planes, then compares every
planar sample back to the interleaved source. Its JSON stdout records schema
version, asset ID, source/prepared/output digests, bytes, relative output name,
layout and recipe provenance. Retain stdout as generator provenance in the
approved store. This additional derivative does not change `prepared.json` or
replace the interleaved codec comparison reference.

## Calibration evidence

[The factual preparation record](rareplanes-calibration-v1.evidence.json)
contains no pixel arrays or host paths. All eight complete rasters passed the
independent sample comparison. Magadino is 5329 × 2878 for PAN/RGB8 and
1332 × 720 for MS/RGB16; Apple Valley is 6650 × 7054 and 1663 × 1763.
The eight raw assets total 396,829,808 bytes. All declared nodata counts were
zero. No U16 value reached 65535; observed U16 values reached 10000 in Magadino.
RGB8 storage-maximum counts are recorded per band rather than treated as new
clipping introduced by preparation.

The host's initial GDAL spatial-reference reads warned that its PROJ database
could not be opened. The TIFF spatial references were still available and no
reprojection was requested; every source-to-output sample compared exactly.
Synthetic tests use a project-authored local coordinate system and do not need
an EPSG database. Geodetic accuracy is not an acceptance claim of this recipe.

Offline validation passed eight project-authored GDAL tests covering band order,
U16 byte order and range, U8 values, nodata, invalid formats/types, scale metadata,
overwrite/path refusal, planar layout and lock/inventory agreement. The existing
37 Rust tests and catalogue checks also passed. Source verification covered all
eight upstream objects (373,714,259 bytes) and their complete tree digest.

Retain this preparation for bounded codec calibration. It proves unchanged
full-image samples and identities. It does not prove eight-component codec
admission, lossless round trips, benchmark performance, geodetic accuracy or
selection representativeness; those claims have separate owners and evidence.
