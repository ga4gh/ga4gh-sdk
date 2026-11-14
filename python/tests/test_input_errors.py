"""Errors from Python bindings should stay normal Python exceptions."""

import os
import tempfile
import unittest
from unittest.mock import patch

from ga4gh_sdk import Configuration, ServiceType, Transport


class InputErrorsTest(unittest.TestCase):
    def test_invalid_base_path(self):
        with self.assertRaisesRegex(ValueError, "Invalid base_path"):
            Configuration("not a URL")

        config = Configuration("https://example.org/tes/v1")
        with self.assertRaisesRegex(ValueError, "Invalid base_path"):
            config.set_base_path("not a URL")
        self.assertEqual(config.get_base_path(), "https://example.org/tes/v1")

    def test_invalid_json(self):
        transport = Transport(Configuration("http://127.0.0.1:1"))
        cases = [
            ("get", ("tasks", "{"), "Invalid params JSON"),
            ("post", ("tasks", "{"), "Invalid data JSON"),
            ("put", ("tasks/1", "{"), "Invalid data JSON"),
        ]
        for method, args, message in cases:
            with self.subTest(method=method):
                with self.assertRaisesRegex(ValueError, message):
                    getattr(transport, method)(*args)

    @unittest.skipUnless(os.name == "posix", "HOME controls the config path on Unix")
    def test_missing_configuration_names_file(self):
        with tempfile.TemporaryDirectory() as home, patch.dict("os.environ", {"HOME": home}):
            config = Configuration("https://example.org")
            with self.assertRaisesRegex(FileNotFoundError, r"\.ga4gh-cli/config.json"):
                config.from_file(ServiceType.TES)

    @unittest.skipUnless(os.name == "posix", "HOME controls the config path on Unix")
    def test_malformed_configuration_is_value_error(self):
        with tempfile.TemporaryDirectory() as home, patch.dict("os.environ", {"HOME": home}):
            config_dir = os.path.join(home, ".ga4gh-cli")
            os.mkdir(config_dir)
            with open(os.path.join(config_dir, "config.json"), "w") as output:
                output.write("not json")
            config = Configuration("https://example.org")
            with self.assertRaisesRegex(ValueError, "Invalid JSON"):
                config.from_file(ServiceType.TES)


if __name__ == "__main__":
    unittest.main()
