# RarePlanes twelve-location preparation, version 1

This local benchmark selection expands the preserved
[two-location calibration](rareplanes-calibration-v1.md) to twelve distinct
WorldView-3 airfields. It is a metadata-coverage sample, not a statistically
representative estimate of RarePlanes or satellite imagery generally. The
selection was frozen before source acquisition and codec timing, retaining both
original anchors. Metadata contains no image dimensions: size was observed only
after selection and did not influence admission. All complete source products
are preserved regardless of size or later codec success.

## Reproducible selection

[The selection evidence](rareplanes-expanded-v1.selection.json) records the
unchanged CSV identity, all selected metadata, population and sample coverage,
and every greedy selection distance. The population has 253 acquisitions,
112 locations and 21 countries, all labelled WV03. Quotas are nine clear, two
haze and one snow acquisition, with eight USA and four non-USA locations. They
broaden the calibration's weather coverage without making rare weather dominate.
All twelve selected acquisitions happen to be in the upstream training split;
train/test membership was read from the CSV and was not a selection criterion.

Starting with Magadino and Apple Valley, fill snow, haze, then clear quotas.
Candidates must use a new `loc_id` and respect geography caps. Maximise the
minimum distance to existing choices using the equally weighted mean of three
absolute population-midrank-percentile differences (sun elevation, maximum
off-nadir angle and maximum PAN resolution) plus country and state/province
mismatch indicators. Midranks use `(number below + (number tied - 1)/2)/(N-1)`;
exact rational arithmetic removes floating tie ambiguity. Break score ties by
ascending `image_id`. The fixed population satisfies all quotas; failure to
complete them is an error. This deterministic diversity heuristic deliberately
emphasises metadata contrast and does not assign statistical sampling weights.

```sh
python3 recipes/rareplanes-expanded-selection-v1.py \
  --metadata "$RAREPLANES_STORE/source/real/metadata_annotations/RarePlanes_Public_Metadata.csv"
```

## Rights and immutable sources

The user's 9 September 2026 request authorised expanded acquisition and local
sample derivatives within the new persistent store. The live bucket notice was
checked against the previously reviewed SHA-256
`f627ad059128fa5246a21e25759c1d33e35c4bb6287d636c4b970f7df57e7eba`
before any imagery was acquired. Retain that complete CC BY-SA 4.0 notice and
attribute J. Shermeyer, T. Hossler, A. Van Etten, D. Hogan, R. Lewis and D. Kim;
In-Q-Tel - CosmiQ Works and AI.Reverie; RarePlanes Dataset, June 2020.
The unchanged notice, metadata CSV and 36 full TIFF products have exact URL,
ETag, byte length and SHA-256 records in
[the source lock](rareplanes-expanded-v1.sources.json). No pixels enter Git.

The [catalogue manifest](../manifests/common/rareplanes-expanded.toml) remains
`reviewed`, outside qualification suites. The direct-object source inventory
has exact integrity, but formal `locked` admission currently requires an archive.
This benchmark requires neither qualification membership nor a schema change;
formal direct-object admission is separate work. Acquisition is a separately
authorised operation, not a feature of this offline preparation recipe.

## Sample contract and execution

Use Python with GDAL 3.13.0 and NumPy 2.5.1. Run preparation from a clean committed
catalogue checkout and an approved persistent store containing `source/`.
Existing outputs are refused; choose a fresh direct child name for a new run.
The original calibration directory and its objects remain unchanged.

```sh
cargo run -p emuella-corpus -- verify common/rareplanes-expanded \
  --root "$RAREPLANES_STORE/source"
python3 recipes/check-rareplanes.py
python3 recipes/check-rareplanes-expanded.py
python3 recipes/rareplanes-expanded-v1.py prepare \
  --store "$RAREPLANES_STORE" --output-name prepared-candidate
python3 recipes/rareplanes-expanded-v1.py planar \
  --store "$RAREPLANES_STORE" --prepared-name prepared-candidate \
  --asset-id 103_104001004890F500-MS16 --output-name example-MS16.rawl
```

Each acquisition yields full PAN16 (PAN band 1), RGB8 (PS-RGB bands 1,2,3),
MS16 (MS bands 1 through 8) and RGB16 (MS bands 5,3,2). The latter mapping is
explicitly inferred from the documented WV03 VNIR order, bound to every selected
CSV sensor identity and exact source digest; MS TIFFs do not independently name
all spectral bands. The source lock retains the reviewed band-evidence links.
Preparation verifies full geometry, unsigned stored types, band count, identity
scale/offset and PS-RGB colour tags. It preserves nodata and all integer values,
with no crop, stretch, masking, scaling, resampling or new pansharpening. Stored
U16 precision does not claim 16-bit sensor acquisition. Storage-maximum counts
are not inferred sensor saturation.

`prepared.json` retains schema version 1 and the existing interleaved consumer
contract, with 48 assets and pack ID `common/rareplanes-expanded`. Provenance
identifies the catalogue revision, recipe, unchanged shared sample writer,
selection algorithm, selection record, source lock and exact tool versions.
The existing sample writer independently reopens TIFFs with per-band
`ReadRaster`, using different block boundaries, and compares every output
sample. Statistics include nodata. Planar mode copies unchanged little-endian
sample words and compares every planar sample to the interleaved input; retain
its JSON stdout in the approved store. These checks establish source equality,
not codec round-trip, performance, geodetic accuracy or generalisation claims.
