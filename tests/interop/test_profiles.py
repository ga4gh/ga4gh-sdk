import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

import run


class Reports(unittest.TestCase):
    def test_every_classification_survives_junit(self):
        checks = [run.item(x,x,x) for x in sorted(run.OUTCOMES)]
        report = {'profiles':[{'id':'synthetic','evidence':'simulated_protocol','checks':checks}]}
        xml = ET.fromstring(run.junit(report))
        self.assertEqual(xml.attrib, {'name':'ga4gh-provider-profiles','tests':'7','failures':'3','errors':'1','skipped':'2'})
        self.assertEqual({c.find('properties/property').attrib['value'] for c in xml}, run.OUTCOMES)
        self.assertEqual({c.find('properties/property[@name="evidence"]').attrib['value'] for c in xml}, {'simulated_protocol'})

    def test_verified_fixture_seed_and_changed_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            self.assertEqual(run.seed_fixtures(Path(temp))[-1]['outcome'], 'passed')
            self.assertEqual((Path(temp)/'seed.txt').read_bytes(), (run.ROOT/'fixtures/seed.txt').read_bytes())

    def test_corrupt_or_missing_seed_is_not_a_provider_defect(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixtures = root/'fixtures'
            fixtures.mkdir()
            manifest = json.loads((run.ROOT/'fixtures/manifest.json').read_text())
            (fixtures/'manifest.json').write_text(json.dumps(manifest))
            with patch.object(run, 'ROOT', root):
                self.assertEqual(run.seed_fixtures(root/'seeded')[0]['outcome'], 'missing_fixture')
                for fixture in manifest['fixtures']:
                    (fixtures/fixture['path']).write_bytes(b'changed content')
                self.assertEqual(run.seed_fixtures(root/'seeded')[0]['outcome'], 'missing_fixture')

    def test_invalid_outcome_is_rejected(self):
        with self.assertRaises(ValueError):
            run.item('x','certified-compatible','x')

    def test_profiles_have_pins_and_private_context_is_absent(self):
        for path in (run.ROOT/'profiles').glob('*.json'):
            profile = json.loads(path.read_text())
            for key in ['id','spec','implementation','client','fixtures','authentication','storage','features','api_root']:
                self.assertIn(key, profile, path)
            self.assertFalse(any(key in profile for key in ['token','workspaceId','computeEnvId']))


if __name__ == '__main__':
    unittest.main()
