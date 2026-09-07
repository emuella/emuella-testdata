#!/usr/bin/env python3
"""Check the committed independent NITF pack without NumPy or GDAL bindings.

Optionally run a complete separate OpenJPEG CLI decode into explicit scratch.
"""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
PACK = ROOT / 'generated/jpeg-2000/independent-nitf'
SPEC = importlib.util.spec_from_file_location('recipe', Path(__file__).with_name('independent-nitf-v1.py'))
RECIPE = importlib.util.module_from_spec(SPEC)
sys.dont_write_bytecode = True
SPEC.loader.exec_module(RECIPE)


def oracle(x, y, band, bits, bands):
    maximum = (1 << bits) - 1
    value = (x * (3 + band) + y * (5 + 2 * band)) % (maximum + 1)
    if 41 <= x % 113 < 48 and 23 <= y % 61 < 28:
        value = maximum * (band + 1) // (bands + 1)
    if (x % 127 - 83) ** 2 + (y % 67 - 37) ** 2 <= 16:
        value = maximum
    if (x + 3 * y + band) % 47 == 0:
        value += 1
    if x % 251 == 0:
        value = 0
    return max(0, min(maximum, value))


def read_pnm(path):
    with path.open('rb') as stream:
        magic = stream.readline().strip()
        words = []
        while len(words) < 3:
            line = stream.readline()
            if not line:
                raise ValueError('Incomplete PNM header')
            if not line.startswith(b'#'):
                words.extend(line.split())
        width, height, maximum = map(int, words)
        payload = stream.read()
    bands = 1 if magic == b'P5' else 3 if magic == b'P6' else 0
    assert bands, magic
    size = 1 if maximum < 256 else 2
    assert len(payload) == width * height * bands * size
    values = list(payload) if size == 1 else [value[0] for value in struct.iter_unpack('>H', payload)]
    return width, height, bands, values


def check(opj_decompress=None, work=None):
    provenance = json.loads((PACK / 'PROVENANCE.json').read_text())
    manifest = tomllib.loads((ROOT / 'manifests/jpeg-2000/independent-nitf.toml').read_text())
    assert provenance['recipe_sha256'] == RECIPE.sha256(Path(RECIPE.__file__))
    assert provenance['openjpeg_version'] == '2.5.4'
    assert {case['id'] for case in provenance['cases']} == set(RECIPE.CASES)
    assert {asset['path'] for asset in manifest['assets']} == {path.name for path in PACK.iterdir()}
    for asset in manifest['assets']:
        path = PACK / asset['path']
        assert asset['bytes'] == path.stat().st_size, path
        assert asset['sha256'] == RECIPE.sha256(path), path
    for case in provenance['cases']:
        name, width, height = case['id'], case['width'], case['height']
        bits, bands = case['bits'], case['bands']
        assert tuple((width, height, bits, bands, case['lossy'])) == RECIPE.CASES[name]
        parameters = RECIPE.coding_parameters(PACK / (name + '.j2k'))
        assert parameters == case['coding']
        assert parameters['precision'] == [bits] * bands
        assert parameters['signed'] == [False] * bands
        assert (parameters['tile_width'], parameters['tile_height']) == (1024, 1024)
        assert (parameters['decompositions'], parameters['progression'], parameters['mct']) == (5, 0, 0)
        assert parameters['layers'] == (19 if case['lossy'] else 20)
        assert parameters['precincts'] == [[32768, 32768]] * 6
        assert parameters['reversible'] == (not case['lossy'])
        assert (parameters['codeblock_width'], parameters['codeblock_height']) == (64, 64)
        for filename, identity in case['files'].items():
            assert identity == dict(bytes=(PACK / filename).stat().st_size,
                                    sha256=RECIPE.sha256(PACK / filename))
        # Verify NITF's extent gives exactly the locked raw codestream.
        nitf = (PACK / (name + '.ntf')).read_bytes()
        assert nitf[:9] == b'NITF02.10' and nitf[360:363] == b'001'
        assert int(nitf[342:354]) == len(nitf)
        start = int(nitf[354:360]) + int(nitf[363:369])
        end = start + int(nitf[369:379])
        assert nitf[start:end] == (PACK / (name + '.j2k')).read_bytes()
        # Calculate every source sample without importing the NumPy generator.
        digest = hashlib.sha256()
        for top in range(0, height, 1024):
            for left in range(0, width, 1024):
                for y in range(top, min(top + 1024, height)):
                    for x in range(left, min(left + 1024, width)):
                        for band in range(bands):
                            value = oracle(x, y, band, bits, bands)
                            digest.update(value.to_bytes(1 if bits == 8 else 2, 'little'))
        assert digest.hexdigest() == case['source_pixels_sha256'], name
        if not case['lossy']:
            assert case['peak_error'] == case['squared_error'] == 0
            assert case['openjpeg_decoded_pixels_sha256'] == digest.hexdigest()
        if opj_decompress:
            destination = work / (name + ('.pgm' if bands == 1 else '.ppm'))
            subprocess.run([str(opj_decompress), '-i', str(PACK / (name + '.j2k')),
                            '-o', str(destination)], check=True, capture_output=True)
            dw, dh, db, actual = read_pnm(destination)
            assert (dw, dh, db) == (width, height, bands)
            peak, squared = 0, 0
            for y in range(height):
                for x in range(width):
                    for band in range(bands):
                        delta = actual[(y * width + x) * bands + band] - oracle(x, y, band, bits, bands)
                        peak = max(peak, abs(delta))
                        squared += delta * delta
            # These are observed complete-decode errors for the committed
            # profile, not general visually-lossless acceptance thresholds.
            assert peak == case['peak_error'], (name, peak)
            assert squared == case['squared_error'], (name, squared)
        print(f'{name}: integrity, coding parameters and complete source oracle pass'
              + ('; complete OpenJPEG CLI decode agrees' if opj_decompress else ''))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--opj-decompress', type=Path)
    parser.add_argument('--work', type=Path)
    args = parser.parse_args()
    if bool(args.opj_decompress) != bool(args.work):
        parser.error('--opj-decompress and --work must be supplied together')
    if args.work:
        args.work.mkdir(parents=True, exist_ok=False)
    check(args.opj_decompress, args.work)
