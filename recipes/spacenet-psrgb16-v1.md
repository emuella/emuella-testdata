# SpaceNet supplier-pansharpened RGB16 chips, version 1

`common/spacenet-psrgb16`, version `1`, is a separate native high-bit-depth
supplier-pansharpened RGB class. It contains sixteen complete original GeoTIFF
chips from the official SN3 roads sample: four each from Vegas, Paris, Shanghai
and Khartoum. It does not change RarePlanes or claim full-scene scalability,
vendor JPEG 2000 codestream coverage or statistically representative performance.

## Selection and source identity

[The frozen selection](spacenet-psrgb16-v1.selection.json) predates source sample
inspection and codec results. Within each AOI, order the ten original
`RGB-PanSharpen` member names lexically and take ranks 0, 3, 6 and 9. The
[source lock](spacenet-psrgb16-v1.sources.json) retains all forty eligible names,
sixteen selected member hashes and byte lengths, archive identity, source
metadata, AOI catalogue IDs and rights/origin evidence. Do not replace members
because of compression, visual quality or runtime outcomes.

The archive is `SN3_roads_sample.tar.gz`, 764,157,199 bytes, SHA-256
`e30f7192ed8f37227fefde12cd3431ed14e4a174a056b983d00632ed42a5f26d`.
Its complete scan found 237 entries, 1,691,632,905 regular-member bytes and no
embedded licence/attribution notices. Retain the unchanged separately supplied
bucket `LICENSE.md`. The eighteen-file source inventory includes that notice,
the preserved archive and sixteen unchanged selected TIFFs under their original
member paths. These satisfy archive-backed locked admission; this optional pack
is outside all qualification suites. Acquisition is a separately authorised
operation and is not implemented by the preparation script.

Khartoum has role `reserved`; the other AOIs have role `development`. Reserve
permits source validation and untimed lossless exactness only. Exclude the whole
Khartoum group from lossy and performance evaluation. Keep related SN2/SN3 and
AOI full products grouped; per-chip parent tile mapping is unknown and catalogue
identity alone does not establish footprint-level independence. This is a
geographic reserve policy, not a proved independent holdout. Upstream train/test
membership is unknown for these sample-archive chips.

## Origin, rights and attribution

The release authors describe native high-bit RGB-PanSharpen imagery separately
from subsequent display conversion in section 2, “16-bit Imagery Conversion”, of
[their source preparation article](https://medium.com/the-downlinq/creating-training-datasets-for-the-spacenet-road-detection-and-routing-challenge-6f970d413e2f).
These are the original RGB-PanSharpen members, not RGB8 upcasts, native MSI RGB,
new pansharpening or crops assembled by Emuella. All sixteen selected sources
have 1,300 × 1,300 pixels, three UInt16 bands and explicit Red/Green/Blue tags.
Stored precision and observed maxima do not establish effective sensor precision;
the source contract leaves undeclared sensor precision unknown.

The [bucket licence](https://spacenet-dataset.s3.amazonaws.com/LICENSE.md)
explicitly covers all contents except `Hosted-Datasets`, which does not contain
these selected SN3 sources. The release is CC BY-SA 4.0. Attribute SpaceNet
Dataset, SpaceNet Partners and DigitalGlobe imagery; cite Van Etten, Lindenbaum
and Bacastow (2018), *SpaceNet: A Remote Sensing Dataset and Challenge Series*.
Retain the notice, source lineage and modification notices; sharing adaptations
invokes ShareAlike. This recipe produces local technical sample/mask derivatives
only. Image redistribution, training and publication of derivatives are outside
its operational scope. The Apache-2.0 catalogue does not relicense source pixels.

## Native samples and validity

Use GDAL 3.13.0 and NumPy 2.5.1 for preparation from a clean committed catalogue
checkout. Ordinary metadata tests and catalogue verification require neither;
the authored raster probes additionally run when both optional packages exist.
Set `SPACENET_STORE` to the approved persistent store containing the locked
`source/` tree. The script performs no network access and refuses existing
outputs, symlinks, incomplete trees, metadata drift and wrong source types.

```sh
python3 recipes/check-spacenet-psrgb16.py
cargo run -p emuella-corpus -- verify common/spacenet-psrgb16 \
  --root "$SPACENET_STORE/source"
python3 recipes/spacenet-psrgb16-v1.py prepare \
  --store "$SPACENET_STORE" --output-name prepared-candidate
python3 recipes/spacenet-psrgb16-v1.py verify \
  --store "$SPACENET_STORE" --output-name prepared-candidate
```

`prepared.json` keeps schema version 1 and the established packed pixel-interleaved
unsigned little-endian consumer contract: `product=RGB16`, `precision=16`,
`source_kind=supplier-pansharpened-rgb16`, source TIFF stem as asset ID, and AOI as
bundle/acquisition group. Each raw file has 10,140,000 bytes. Each asset's
`validity` record names and hashes a 5,070,000-byte interleaved three-band uint8
array: 1 is valid, 0 invalid. Per band, validity means its GDAL mask is nonzero
and its integer sample differs from declared nodata. Annotation masks are never
used. The selected sources currently declare no nodata and all-valid masks;
zeros remain valid. Tests independently cover nodata and explicit masks.

Preserve every integer including invalid samples. Do not mask, crop, normalise,
scale, stretch, resample or repansharpen. Retain source scale/offset unapplied,
CRS WKT, affine transform, pixel basis vectors in CRS units, dimensions, colour
order, stored types, descriptions and source metadata. Statistics include invalid
samples and distinguish zero, nodata and storage-maximum counts. Sensor precision
is separate and unknown when undeclared; maximum counts do not prove clipping.

The shared native calibration helpers supply hashing, safe new child names and
JSON writing. The specialised writer is necessary because the original
RarePlanes writer rejects nonidentity scale/offset and does not write validity.
It uses dataset array reads, then independently reopens the source and both
outputs and compares every value through per-band `ReadRaster` with different
block boundaries. `verify` repeats source locks and every sample/mask comparison.
No RarePlanes preparation code changes. Provenance records exact recipe/helper,
selection/source-lock hashes, committed catalogue revision and tool versions.
These are source-exactness checks, not codec or performance evidence.
