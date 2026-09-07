# Independent arithmetic NITF fixtures, version 1

`independent-nitf-v1.py` authors every source sample from integer coordinates.
It generates ramps, edges, small rectangles and circles, and one-code-value
perturbations. No satellite image, third-party pixel, font, colour profile,
protected source, or derivative contributes to the imagery. The recipe and
resulting fixtures are project-authored Apache-2.0 material under the root
licence. OpenJPEG and GDAL are independent encoding/decoding tools; their
software licences do not replace the imagery's provenance.

The committed pack is `jpeg-2000/independent-nitf`, version `1`. It contains five
NITF 2.1 C8 images and each image's byte-identical extracted raw codestream:

| Case | Dimensions | Samples | Coding |
|---|---|---|---|
| `pan11-lossless` | 1057 × 65 | unsigned 11-bit grey in U16 | reversible, 20 layers |
| `pan11-lossy` | 1057 × 65 | unsigned 11-bit grey in U16 | irreversible, 19 layers |
| `grey16-lossless` | 97 × 65 | unsigned 16-bit grey | reversible, 20 layers |
| `rgb8-lossless` | 97 × 65 | unsigned 8-bit RGB | reversible, 20 layers |
| `rgb16-lossless` | 97 × 65 | unsigned 16-bit RGB | reversible, 20 layers |

All observed codestreams use 1024 × 1024 tiles, five decompositions, 64 × 64
code blocks, LRCP progression, implicit 32768 × 32768 precincts at every
resolution and no multiple-component transform. The PAN
cases cross a tile boundary. The GDAL creation options
`NPJE_NUMERICALLY_LOSSLESS` and `NPJE_VISUALLY_LOSSLESS` select the independent
encoder settings. Those option names are provenance, **not a claim of NPJE
conformance**, real-world satellite equivalence, or any viewer's performance.
The observed progression is LRCP, not RLCP. These fixtures do not qualify an
original WorldView product or replace its separate rights and acceptance work.

The 11-bit source is created as a tiled UInt16 GeoTIFF with `NBITS=11` and is
encoded with NITF `NBITS=11`. The recipe checks both NITF `ABPP=11` and actual
codestream SIZ precision 11. It does not patch encoded headers or encode
16-bit samples and relabel them afterwards. Fixed NITF dates, identifiers,
single-threaded execution and explicit colour interpretations remove implicit
timestamp and backend choices.

## Dependencies and regeneration

The checked-in provenance records GDAL `3.14.0dev-1af54d9995`, source revision
`1af54d99959f3b62ba10451a357a969075374663`, OpenJPEG `2.5.4`, NumPy `2.5.1`,
the recipe SHA-256 and all creation options. The GDAL source revision belongs
to the maintained [Emuella GDAL fork](https://github.com/emuella/gdal/tree/1af54d99959f3b62ba10451a357a969075374663).
Generation requires a little-endian host, Python 3.11 or newer, NumPy, and that GDAL shared library
built with GTiff, NITF and JP2OpenJPEG. The recipe calls the public C API via
`ctypes`, so matching GDAL Python bindings are unnecessary. Only those three
drivers are registered; no Emuella or proprietary encoder is selected.

Set `GDAL_LIBRARY` to the built shared library and `SCRATCH` to a caller-owned
generated-output directory. The library's runtime dependencies must be on the
loader search path. Each output/work directory must be new:

```sh
python3 recipes/independent-nitf-v1.py \
  --gdal-library "$GDAL_LIBRARY" \
  --gdal-source-revision 1af54d99959f3b62ba10451a357a969075374663 \
  --output "$SCRATCH/first" --work "$SCRATCH/first-work"
python3 recipes/independent-nitf-v1.py \
  --gdal-library "$GDAL_LIBRARY" \
  --gdal-source-revision 1af54d99959f3b62ba10451a357a969075374663 \
  --output "$SCRATCH/second" --work "$SCRATCH/second-work"
diff -r "$SCRATCH/first" "$SCRATCH/second"
diff -r generated/jpeg-2000/independent-nitf "$SCRATCH/first"
```

The source revision argument is the caller's build attestation. The recipe
also records runtime GDAL/OpenJPEG versions; the caller must retain build
configuration and binary identity when making a qualification claim. A changed
toolchain or recipe must be checked against the locked outputs. Changed bytes
require a new pack version.

## Verification and independent pixels

```sh
python3 recipes/check-independent-nitf.py
python3 recipes/check-independent-nitf.py \
  --opj-decompress /path/to/opj_decompress --work "$SCRATCH/openjpeg-check"
cargo run -p emuella-corpus -- verify jpeg-2000/independent-nitf
```

The first check uses only Python's standard library. It verifies catalogue
asset identities, NITF segment extraction, codestream parameters and every
source sample through a separate scalar implementation of the arithmetic
oracle. The second command performs complete OpenJPEG CLI decodes of all five
small images. The locked lossless references have zero error; the lossy PAN
reference has peak error 1 and summed squared error 1995 over 68705 samples.
These values describe this fixture, not a general visual-quality threshold.

Generation independently decodes the extracted raw codestream through
JP2OpenJPEG, compares every native sample to the source oracle, and records
both pixel hashes and aggregate errors in `PROVENANCE.json`. Hash order is
1024-square tiles, top-to-bottom then left-to-right, with row-major interleaved
U8 or little-endian U16 samples within each tile. A consumer can import
`pixels(x, y, width, height, bits, bands)` for any absolute-coordinate source
window; it must use an independent decoded reference for lossy source pixels.

## Larger generated inputs

The same recipe accepts `--case pan11-lossless --width 32768 --height 32768`
or explicit dimensions for another listed case. Large images remain in
caller-owned scratch and are not part of the committed pack or its suite.
The recipe writes a disk-backed tiled GeoTIFF, encodes it tile by tile through
GDAL/OpenJPEG, and verifies the complete codestream in 1024-square windows.
Source generation and pixel comparison allocate only bounded tiles; the GDAL
cache is limited to 64 MiB and both thread settings are one. It neither
constructs an image-sized NumPy array nor invokes the whole-image OpenJPEG CLI
decoder for large inputs. Codec metadata and disk usage still scale with image
and tile count. Resource limits must be measured on the actual target before
claiming an operational bound. Large generation performs no qualification
timing or viewer test itself.

The public APIs and option semantics are documented by
[GDAL NITF](https://gdal.org/en/stable/drivers/raster/nitf.html),
[GDAL JP2OpenJPEG](https://gdal.org/en/stable/drivers/raster/jp2openjpeg.html),
[GDAL raster C API](https://gdal.org/en/stable/api/raster_c_api.html) and
[OpenJPEG](https://www.openjpeg.org/doxygen/openjpeg_8h.html).
