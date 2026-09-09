#!/usr/bin/env python3
"""Offline authored checks for expanded selection and complete product admission."""
import copy
import csv
import hashlib
import io
from unittest.mock import patch
import tomllib
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent

def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

recipe = load('rareplanes-expanded-v1')
selection_recipe = load('rareplanes-expanded-selection-v1')

class ExpandedTests(unittest.TestCase):
    def setUp(self):
        self.lock = json.loads(recipe.LOCK.read_text())
        self.selection = json.loads(recipe.SELECTION.read_text())

    def test_complete_frozen_membership(self):
        recipe.validate_lock(self.lock)
        self.assertEqual(self.selection['selected']['locations'], 12)
        self.assertEqual(self.selection['selected']['weather'], selection_recipe.QUOTAS)
        self.assertEqual(sum(b['metadata']['Country'] == 'USA' for b in self.lock['bundles']), 8)
        self.assertTrue(set(selection_recipe.ANCHORS) <= {b['id'] for b in self.lock['bundles']})
        self.assertEqual(len(recipe.PRODUCTS), 4)

    def test_missing_product_duplicate_location_and_changed_sensor_refused(self):
        mutations = [lambda x: x['assets'].pop(),
                     lambda x: x['bundles'][2]['metadata'].update(loc_id=x['bundles'][0]['metadata']['loc_id']),
                     lambda x: x['bundles'][2]['metadata'].update(sensor='WV02'),
                     lambda x: x['bundles'][2].update(split='test'),
                     lambda x: x['band_evidence'].update(rgb16_indices=[1,2,3]),
                     lambda x: x['bundles'][2].update(pan_dimensions=[0,2]),
                     lambda x: x['assets'][0].update(path='../escape')]
        for mutate in mutations:
            lock = copy.deepcopy(self.lock)
            mutate(lock)
            with self.assertRaises(ValueError):
                recipe.validate_lock(lock)

    def test_authored_population_ties_order_and_unique_locations(self):
        rows = []
        template = dict(self.selection['bundles'][0]['metadata'])
        for i in range(253):
            row = dict(template, image_id=f"authored-{i:03d}", loc_id=str(i),
                       Country='USA' if i < 200 else 'Other', State_Province='Same',
                       Weather='Snow' if i < 20 else ('Cloud Cover or Haze' if i < 50 else 'Clear Skies'))
            rows.append(row)
        rows[100]['image_id'], rows[100]['Country'] = selection_recipe.ANCHORS[0], 'Other'
        rows[101]['image_id'] = selection_recipe.ANCHORS[1]
        def run(items):
            stream = io.StringIO()
            writer = csv.DictWriter(stream, fieldnames=list(template))
            writer.writeheader()
            writer.writerows(items)
            data = stream.getvalue().encode()
            with patch.object(selection_recipe, 'METADATA_SHA256', hashlib.sha256(data).hexdigest()):
                return selection_recipe.select(data)
        forward, reverse = run(rows), run(list(reversed(rows)))
        self.assertEqual(forward['bundles'], reverse['bundles'])
        self.assertEqual(forward['trace'], reverse['trace'])
        self.assertEqual(forward['trace'][0]['image_id'], 'authored-000')
        self.assertEqual(forward['selected']['locations'], 12)
        self.assertEqual(forward['selected']['weather'], selection_recipe.QUOTAS)
        self.assertEqual(sum(b['metadata']['Country'] == 'USA' for b in forward['bundles']), 8)

    def test_inventory_tree_and_source_lock_agree(self):
        inventory = tomllib.loads((recipe.ROOT / 'inventories/common/rareplanes-expanded-v1.toml').read_text())
        manifest = tomllib.loads((recipe.ROOT / 'manifests/common/rareplanes-expanded.toml').read_text())
        actual = {a['path']: (a['bytes'],a['sha256']) for a in self.lock['assets']}
        self.assertEqual(actual, {a['path']: (a['bytes'],a['sha256']) for a in inventory['assets']})
        encoded = ''.join(f"{digest}\t{size}\t{path}\n" for path,(size,digest) in sorted(actual.items()))
        self.assertEqual(hashlib.sha256(encoded.encode()).hexdigest(), manifest['materialization']['expected_tree_sha256'])
        self.assertEqual(manifest['review_state'], 'reviewed')

    def test_metadata_mutation_refused(self):
        with self.assertRaises(ValueError):
            selection_recipe.select(b'changed metadata')

    def test_source_identities_and_attribution(self):
        assets = {a['path']:a for a in self.lock['assets']}
        self.assertEqual(assets['LICENSE.txt']['sha256'], 'f627ad059128fa5246a21e25759c1d33e35c4bb6287d636c4b970f7df57e7eba')
        self.assertEqual(assets['real/metadata_annotations/RarePlanes_Public_Metadata.csv']['sha256'], selection_recipe.METADATA_SHA256)
        for path, asset in assets.items():
            self.assertEqual(asset['url'], 'https://rareplanes-public.s3.us-west-2.amazonaws.com/' + path)
            self.assertEqual(len(asset['sha256']),64)
            self.assertGreater(asset['bytes'],0)
            self.assertTrue(asset['etag'])

if __name__ == '__main__':
    unittest.main()
