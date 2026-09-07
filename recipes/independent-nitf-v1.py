#!/usr/bin/env python3
"""Generate Apache-2.0 arithmetic imagery through GDAL/OpenJPEG's public C API.

Requires Python 3.11+, NumPy and a GDAL shared library with GTiff, NITF and
JP2OpenJPEG. No imagery is read as a source. See independent-nitf-v1.md.
"""

import argparse
import ctypes as C
import hashlib
import json
from pathlib import Path
import struct
import sys


TILE = 1024
CASES = {
    "pan11-lossless": (1057, 65, 11, 1, False),
    "pan11-lossy": (1057, 65, 11, 1, True),
    "grey16-lossless": (97, 65, 16, 1, False),
    "rgb8-lossless": (97, 65, 8, 3, False),
    "rgb16-lossless": (97, 65, 16, 3, False),
}


def pixels(x, y, width, height, bits, bands):
    """Pixel-interleaved integer oracle; coordinates are absolute, never seeded."""
    import numpy as np

    xx = np.arange(x, x + width, dtype=np.int64)[None, :]
    yy = np.arange(y, y + height, dtype=np.int64)[:, None]
    maximum = (1 << bits) - 1
    channels = []
    for band in range(bands):
        # Smooth terrain, hard edges, small rectangular/circular objects and
        # one-code-value perturbations. All shapes repeat at fixed coordinates.
        base = (xx * (3 + band) + yy * (5 + 2 * band)) % (maximum + 1)
        rectangle = ((xx % 113 >= 41) & (xx % 113 < 48)
                     & (yy % 61 >= 23) & (yy % 61 < 28))
        circle = ((xx % 127 - 83) ** 2 + (yy % 67 - 37) ** 2 <= 16)
        base = np.where(rectangle, maximum * (band + 1) // (bands + 1), base)
        base = np.where(circle, maximum, base)
        base = np.where((xx + 3 * yy + band) % 47 == 0, base + 1, base)
        base = np.where(xx % 251 == 0, 0, base)
        channels.append(np.clip(base, 0, maximum))
    return np.stack(channels, axis=-1).astype(np.uint8 if bits == 8 else '<u2')


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


class Gdal:
    """Small typed boundary over the documented, stable GDAL C API."""

    def __init__(self, library):
        self.lib = C.CDLL(str(library))
        signatures = {
            'GDALVersionInfo': (C.c_char_p, [C.c_char_p]),
            'CPLSetConfigOption': (None, [C.c_char_p, C.c_char_p]),
            'CPLGetLastErrorMsg': (C.c_char_p, []),
            'GDALSetCacheMax64': (None, [C.c_int64]),
            'GDALGetDriverByName': (C.c_void_p, [C.c_char_p]),
            'GDALCreate': (C.c_void_p, [C.c_void_p, C.c_char_p, C.c_int,
                                      C.c_int, C.c_int, C.c_int,
                                      C.POINTER(C.c_char_p)]),
            'GDALCreateCopy': (C.c_void_p, [C.c_void_p, C.c_char_p, C.c_void_p,
                                          C.c_int, C.POINTER(C.c_char_p),
                                          C.c_void_p, C.c_void_p]),
            'GDALOpen': (C.c_void_p, [C.c_char_p, C.c_int]),
            'GDALClose': (C.c_int, [C.c_void_p]),
            'GDALGetRasterBand': (C.c_void_p, [C.c_void_p, C.c_int]),
            'GDALSetRasterColorInterpretation': (C.c_int, [C.c_void_p, C.c_int]),
            'GDALRasterIO': (C.c_int, [C.c_void_p, C.c_int, C.c_int, C.c_int,
                                      C.c_int, C.c_int, C.c_void_p, C.c_int,
                                      C.c_int, C.c_int, C.c_int, C.c_int]),
            'GDALGetMetadataItem': (C.c_char_p, [C.c_void_p, C.c_char_p, C.c_char_p]),
            'opj_version': (C.c_char_p, []),
        }
        for name, (result, arguments) in signatures.items():
            function = getattr(self.lib, name)
            function.restype, function.argtypes = result, arguments
        # Explicit registration prevents an installed proprietary or Emuella
        # plugin from contributing either encoded bytes or reference pixels.
        for name in ('GTiff', 'JP2OpenJPEG', 'NITF'):
            getattr(self.lib, 'GDALRegister_' + name)()
            if not self.lib.GDALGetDriverByName(name.encode()):
                raise RuntimeError('Missing required GDAL driver: ' + name)
        self.lib.CPLSetConfigOption(b'GDAL_NUM_THREADS', b'1')
        self.lib.CPLSetConfigOption(b'OPJ_NUM_THREADS', b'1')
        self.lib.CPLSetConfigOption(b'GDAL_PAM_ENABLED', b'NO')
        self.lib.GDALSetCacheMax64(64 * 1024 * 1024)

    def checked(self, value):
        if not value:
            raise RuntimeError(self.lib.CPLGetLastErrorMsg().decode())
        return value

    def error(self, code):
        if code != 0:
            raise RuntimeError(self.lib.CPLGetLastErrorMsg().decode())

    @staticmethod
    def options(values):
        return (C.c_char_p * (len(values) + 1))(
            *(value.encode() for value in values), None)

    def create(self, path, width, height, bits, bands):
        dataset = self.checked(self.lib.GDALCreate(
            self.lib.GDALGetDriverByName(b'GTiff'), str(path).encode(),
            width, height, bands, 1 if bits == 8 else 2,
            self.options(['TILED=YES', 'BLOCKXSIZE=1024', 'BLOCKYSIZE=1024',
                          'COMPRESS=NONE', 'BIGTIFF=IF_SAFER', f'NBITS={bits}'])))
        for band in range(bands):
            self.error(self.lib.GDALSetRasterColorInterpretation(
                self.lib.GDALGetRasterBand(dataset, band + 1),
                1 if bands == 1 else 3 + band))
        return dataset

    def io(self, dataset, x, y, data, write=False):
        height, width, bands = data.shape
        for band in range(bands):
            self.error(self.lib.GDALRasterIO(
                self.lib.GDALGetRasterBand(dataset, band + 1), int(write),
                x, y, width, height, data.ctypes.data + band * data.itemsize,
                width, height, 1 if data.itemsize == 1 else 2,
                data.itemsize * bands, data.itemsize * bands * width))

    def open(self, path):
        return self.checked(self.lib.GDALOpen(str(path).encode(), 0))

    def close(self, dataset):
        self.error(self.lib.GDALClose(dataset))

    def metadata(self, dataset, key):
        value = self.lib.GDALGetMetadataItem(dataset, key.encode(), None)
        return value.decode() if value else None


def extract_codestream(nitf, destination):
    """Read the first segment's declared extent; never search for marker bytes."""
    with nitf.open('rb') as source, destination.open('xb') as output:
        header = source.read(379)
        if header[:9] != b'NITF02.10' or header[360:363] != b'001':
            raise ValueError('Expected NITF 2.1 with exactly one image segment')
        header_length = int(header[354:360])
        subheader_length = int(header[363:369])
        image_length = int(header[369:379])
        if int(header[342:354]) != nitf.stat().st_size:
            raise ValueError('NITF file length mismatch')
        source.seek(header_length + subheader_length)
        remaining = image_length
        while remaining:
            block = source.read(min(remaining, 1024 * 1024))
            if not block:
                raise ValueError('Truncated NITF image segment')
            output.write(block)
            remaining -= len(block)


def coding_parameters(path):
    """Inspect SIZ/COD before the first tile without reading pixel payloads."""
    result = {}
    with path.open('rb') as stream:
        if stream.read(2) != b'\xff\x4f':
            raise ValueError('Expected raw JPEG 2000 codestream')
        while True:
            marker = stream.read(2)
            if marker == b'\xff\x90':
                break
            length = struct.unpack('>H', stream.read(2))[0]
            if length < 2:
                raise ValueError('Invalid marker length')
            data = stream.read(length - 2)
            if marker == b'\xff\x51':
                result.update(rsiz=int.from_bytes(data[:2], 'big'),
                              width=int.from_bytes(data[2:6], 'big'),
                              height=int.from_bytes(data[6:10], 'big'),
                              tile_width=int.from_bytes(data[18:22], 'big'),
                              tile_height=int.from_bytes(data[22:26], 'big'),
                              precision=[(value & 127) + 1 for value in data[36::3]],
                              signed=[bool(value & 128) for value in data[36::3]])
            elif marker == b'\xff\x52':
                result.update(progression=data[1], layers=int.from_bytes(data[2:4], 'big'),
                              mct=data[4], decompositions=data[5],
                              codeblock_width=1 << (data[6] + 2),
                              codeblock_height=1 << (data[7] + 2),
                              codeblock_style=data[8], reversible=bool(data[9]))
                result['precincts'] = (
                    [[1 << (value & 15), 1 << (value >> 4)] for value in data[10:]]
                    if data[0] & 1 else [[32768, 32768]] * (data[5] + 1))
    return result


def generate_case(gdal, output, work, name, dimensions=None):
    import numpy as np

    width, height, bits, bands, lossy = CASES[name]
    if dimensions:
        width, height = dimensions
    source_path = work / (name + '.tif')
    source = gdal.create(source_path, width, height, bits, bands)
    try:
        for y in range(0, height, TILE):
            for x in range(0, width, TILE):
                data = pixels(x, y, min(TILE, width - x), min(TILE, height - y), bits, bands)
                gdal.io(source, x, y, data, write=True)
    finally:
        gdal.close(source)
    source = gdal.open(source_path)
    nitf = output / (name + '.ntf')
    options = [
        'IC=C8', 'JPEG2000_DRIVER=JP2OpenJPEG', 'FHDR=NITF02.10',
        'PROFILE=NPJE_VISUALLY_LOSSLESS' if lossy else 'PROFILE=NPJE_NUMERICALLY_LOSSLESS',
        'BLOCKXSIZE=1024', 'BLOCKYSIZE=1024', f'NBITS={bits}',
        'FDT=20000101000000', 'IDATIM=20000101000000',
        'OSTAID=EMUELLA', 'FTITLE=Independent arithmetic fixture v1',
        'ISORCE=Project-authored integer arithmetic',
        'ICAT=VIS', 'IREP=MONO' if bands == 1 else 'IREP=RGB',
        'USE_SRC_NITF_METADATA=NO',
    ]
    try:
        encoded = gdal.checked(gdal.lib.GDALCreateCopy(
            gdal.lib.GDALGetDriverByName(b'NITF'), str(nitf).encode(), source,
            1, gdal.options(options), None, None))
        gdal.close(encoded)
    finally:
        gdal.close(source)
    codestream = output / (name + '.j2k')
    extract_codestream(nitf, codestream)
    parameters = coding_parameters(codestream)
    expected = dict(width=width, height=height, tile_width=TILE, tile_height=TILE,
                    precision=[bits] * bands, signed=[False] * bands,
                    progression=0, decompositions=5, codeblock_width=64,
                    codeblock_height=64, reversible=not lossy,
                    precincts=[[32768, 32768]] * 6)
    for key, value in expected.items():
        if parameters.get(key) != value:
            raise ValueError(f'{name}: {key}: expected {value}, got {parameters.get(key)}')
    if parameters['layers'] < 2:
        raise ValueError('Expected independently encoded quality layers')
    encoded = gdal.open(nitf)
    try:
        metadata = {key: gdal.metadata(encoded, key) for key in
                    ('NITF_FHDR', 'NITF_IC', 'NITF_ABPP', 'NITF_IREP', 'NITF_PVTYPE')}
        if metadata['NITF_IC'] != 'C8' or int(metadata['NITF_ABPP']) != bits:
            raise ValueError('NITF compression or actual precision mismatch')
    finally:
        gdal.close(encoded)
    # A separate OpenJPEG decode of the extracted source codestream supplies
    # the oracle, including every sample in the large tiled profile. Hashes
    # follow a fixed 1024-square tile traversal, rows then columns.
    decoded = gdal.open(codestream)
    source_hash, decoded_hash = hashlib.sha256(), hashlib.sha256()
    peak, squared, samples = 0, 0, 0
    try:
        for y in range(0, height, TILE):
            for x in range(0, width, TILE):
                data = pixels(x, y, min(TILE, width - x), min(TILE, height - y), bits, bands)
                actual = np.empty_like(data)
                gdal.io(decoded, x, y, actual)
                source_hash.update(data.tobytes())
                decoded_hash.update(actual.tobytes())
                delta = actual.astype(np.int64) - data.astype(np.int64)
                peak = max(peak, int(np.max(np.abs(delta))))
                squared += int(np.sum(delta * delta))
                samples += delta.size
    finally:
        gdal.close(decoded)
    if not lossy and peak:
        raise ValueError(f'{name}: independent complete lossless decode disagrees: {peak}')
    if lossy and not peak:
        raise ValueError(f'{name}: lossy probe unexpectedly reproduced every sample')
    return dict(id=name, width=width, height=height, bits=bits, bands=bands,
                lossy=lossy, source_pixels_sha256=source_hash.hexdigest(),
                openjpeg_decoded_pixels_sha256=decoded_hash.hexdigest(),
                peak_error=peak, squared_error=squared, samples=samples,
                coding=parameters, nitf_metadata=metadata, creation_options=options,
                files={path.name: dict(bytes=path.stat().st_size, sha256=sha256(path))
                       for path in (nitf, codestream)})


def main():
    import numpy as np

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gdal-library', required=True, type=Path)
    parser.add_argument('--gdal-source-revision', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--work', required=True, type=Path)
    parser.add_argument('--case', choices=CASES, action='append')
    parser.add_argument('--width', type=int)
    parser.add_argument('--height', type=int)
    args = parser.parse_args()
    if sys.byteorder != 'little':
        parser.error('Version 1 requires a little-endian host for GDAL native U16 buffers')
    if (args.width is None) != (args.height is None):
        parser.error('--width and --height must be supplied together')
    if args.width is not None and (min(args.width, args.height) < 65
                                  or max(args.width, args.height) > 99999999
                                  or ((args.width + 1023) // 1024)
                                  * ((args.height + 1023) // 1024) > 65535):
        parser.error('Dimensions must be at least 65 and admit at most 65535 tiles')
    if len(args.gdal_source_revision) != 40 or any(
            char not in '0123456789abcdef' for char in args.gdal_source_revision):
        parser.error('GDAL source revision must be a full lowercase Git revision')
    args.output.mkdir(parents=True, exist_ok=False)
    args.work.mkdir(parents=True, exist_ok=False)
    gdal = Gdal(args.gdal_library)
    provenance = dict(
        recipe='independent-nitf-v1', licence='Apache-2.0',
        recipe_sha256=sha256(Path(__file__)),
        pixel_order='1024-square tiles top-to-bottom then left-to-right; row-major interleaved U8 or U16 little-endian within each tile',
        gdal_source_revision=args.gdal_source_revision,
        gdal_version=gdal.lib.GDALVersionInfo(b'RELEASE_NAME').decode(),
        openjpeg_version=gdal.lib.opj_version().decode(), numpy_version=np.__version__,
        independent_backend='GDAL NITF CreateCopy / JP2OpenJPEG; separate raw JP2OpenJPEG decode',
        qualification='Arithmetic interoperability fixtures; no NPJE or real-world imagery conformance claim',
        cases=[generate_case(gdal, args.output, args.work, name,
                             (args.width, args.height) if args.width else None)
               for name in (args.case or CASES)],
    )
    (args.output / 'PROVENANCE.json').write_text(
        json.dumps(provenance, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    print(json.dumps(provenance, sort_keys=True))


if __name__ == '__main__':
    main()
