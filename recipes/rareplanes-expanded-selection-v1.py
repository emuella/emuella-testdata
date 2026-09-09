#!/usr/bin/env python3
"""Reproduce the pre-timing RarePlanes metadata-stratified selection offline."""
import argparse
import csv
from fractions import Fraction
import hashlib
import io
import json
from pathlib import Path

METADATA_SHA256 = "005eb9c6c4ab0f0fea1f8402f202066b6f4d29617ef263f72b1be34dea35edec"
ANCHORS = ["30_104001002394E000", "47_104001001D2C7A00"]
FIELDS = ["avg_sun_elevation_angle", "off_nadir_max", "pan_resolution_maximum"]
QUOTAS = {"Snow": 1, "Cloud Cover or Haze": 2, "Clear Skies": 9}
GEOGRAPHY = {"USA": 8, "non-USA": 4}


def select(data):
    if hashlib.sha256(data).hexdigest() != METADATA_SHA256:
        raise ValueError("metadata digest differs from reviewed population")
    rows = list(csv.DictReader(io.StringIO(data.decode())))
    if len(rows) != 253 or len({r['image_id'] for r in rows}) != 253:
        raise ValueError("unexpected population membership")
    if any(r['sensor'] != 'WV03' or (r['Train'], r['Test']) not in [('1', '0'), ('0', '1')] for r in rows):
        raise ValueError("unsupported sensor or split")
    # Population midrank percentile: ties have the same exact rational value.
    ranks = {}
    for field in FIELDS:
        values = [Fraction(r[field]) for r in rows]
        ranks[field] = {v: Fraction(2 * sum(x < v for x in values) + sum(x == v for x in values) - 1,
                                    2 * (len(values) - 1)) for v in set(values)}
    def distance(a, b):
        return (sum(abs(ranks[f][Fraction(a[f])] - ranks[f][Fraction(b[f])]) for f in FIELDS)
                + (a['Country'] != b['Country']) + (a['State_Province'] != b['State_Province'])) / 5
    chosen = [next(r for r in rows if r['image_id'] == key) for key in ANCHORS]
    trace = []
    for weather, quota in QUOTAS.items():
        while sum(r['Weather'] == weather for r in chosen) < quota:
            eligible = [r for r in rows if r['Weather'] == weather
                        and r['loc_id'] not in {s['loc_id'] for s in chosen}
                        and sum((s['Country'] == 'USA') == (r['Country'] == 'USA') for s in chosen)
                        < GEOGRAPHY['USA' if r['Country'] == 'USA' else 'non-USA']]
            if not eligible:
                raise ValueError("selection quotas cannot be completed")
            scores = {r['image_id']: min(distance(r, s) for s in chosen) for r in eligible}
            winner = min(eligible, key=lambda r: (-scores[r['image_id']], r['image_id']))
            chosen.append(winner)
            trace.append({'image_id': winner['image_id'], 'weather': weather,
                          'eligible_candidates': len(eligible), 'minimum_distance': str(scores[winner['image_id']])})
    if len(chosen) != 12 or sum(r['Country'] == 'USA' for r in chosen) != 8:
        raise ValueError("selection quotas differ")
    def coverage(items):
        return {'acquisitions': len(items), 'locations': len({r['loc_id'] for r in items}),
                'countries': sorted({r['Country'] for r in items}),
                'weather': {w: sum(r['Weather'] == w for r in items) for w in QUOTAS},
                'numeric_ranges': {f: [min(float(r[f]) for r in items), max(float(r[f]) for r in items)] for f in FIELDS}}
    return {'schema_version': 1, 'metadata_sha256': METADATA_SHA256,
            'algorithm': 'Anchors first; Snow then Cloud Cover or Haze then Clear Skies. Greedy maximum minimum mean L1 distance over three population midrank percentiles and country/state mismatch indicators, with exact rational arithmetic and ascending image_id ties. One acquisition per loc_id; geography caps 8 USA and 4 non-USA.',
            'anchors': ANCHORS, 'weather_quotas': QUOTAS, 'geography_quotas': GEOGRAPHY,
            'population': coverage(rows), 'selected': coverage(chosen), 'trace': trace,
            'bundles': [{'id': r['image_id'], 'split': 'train' if r['Train'] == '1' else 'test',
                         'location': ', '.join([r['Air_Field'], r['State_Province'], r['Country']]),
                         'metadata': r} for r in chosen]}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--metadata', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(select(args.metadata.read_bytes()), indent=2, sort_keys=True, allow_nan=False))
