import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from package import main, publish


class PackageTest(unittest.TestCase):
    def test_destination_and_cargo_environment_do_not_enter_build_command(self):
        destination = 'output directory; not a command'
        for options, profile in [(['--debug'], []), ([], ['--release'])]:
            with self.subTest(options=options), \
                    patch.dict(os.environ, {'CARGO': 'other-build-tool'}), \
                    patch.object(sys, 'argv', ['package.py', destination, *options]), \
                    patch('package.subprocess.check_output', side_effect=OSError) as build:
                with self.assertRaises(OSError):
                    main()
                build.assert_called_once_with([
                    'cargo', 'build', '--locked', '--message-format=json-render-diagnostics', *profile,
                ], cwd=Path(__file__).resolve().parent, text=True)

    def test_interrupted_rebuild_preserves_published_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = root / 'workflow.so'
            descriptor = root / 'descriptor.json'
            readme = root / 'README.md'
            destination = root / 'package'
            descriptor.write_text('{}')
            readme.write_text('Install the provider.')
            artifact.write_bytes(b'first')
            manifest = publish(destination, artifact, descriptor, readme)
            previous = json.loads(manifest.read_text())
            artifact.write_bytes(b'second')
            replace = Path.replace

            def interrupt(source, target):
                if target == manifest:
                    raise OSError('interrupted before manifest publication')
                return replace(source, target)

            with patch.object(Path, 'replace', interrupt), self.assertRaises(OSError):
                publish(destination, artifact, descriptor, readme)
            self.assertEqual(json.loads(manifest.read_text()), previous)
            publish(destination, artifact, descriptor, readme)
            current = json.loads(manifest.read_text())
            for record, expected in [(previous, b'first'), (current, b'second')]:
                self.assertEqual((destination / record['path']).read_bytes(), expected)
                self.assertEqual(record['digest'], hashlib.sha512(expected).hexdigest())
