"""Shared fixtures: a mock TES-like server in a subprocess and its configuration."""

import base64
import json
import subprocess
import sys
import urllib.request
from pathlib import Path

import pytest

from GA4GH import Configuration


class MockServer:
    """Runs `mock_server.py` in a subprocess and reads back what it received."""

    def __init__(self):
        script = Path(__file__).with_name("mock_server.py")
        self.process = subprocess.Popen(
            [sys.executable, str(script)], stdout=subprocess.PIPE, text=True
        )
        port = int(self.process.stdout.readline())
        self.root = f"http://127.0.0.1:{port}"
        self.base = f"{self.root}/api"

    def last(self):
        with urllib.request.urlopen(f"{self.root}/__requests__", timeout=10) as response:
            request = json.load(response)[-1]
        request["body"] = base64.b64decode(request["body"])
        return request

    def paths(self) -> list[str]:
        """The paths of the requests received so far."""
        with urllib.request.urlopen(f"{self.root}/__requests__", timeout=10) as response:
            return [request["path"] for request in json.load(response)]

    def disconnects(self) -> list[str]:
        """The paths of slow requests whose client closed the connection before the answer."""
        with urllib.request.urlopen(f"{self.root}/__disconnects__", timeout=10) as response:
            return json.load(response)

    def close(self):
        self.process.terminate()
        self.process.wait(timeout=10)
        self.process.stdout.close()


@pytest.fixture(scope="module")
def server():
    mock = MockServer()
    yield mock
    mock.close()


@pytest.fixture
def config(server):
    return Configuration(server.base)


@pytest.fixture
def wes_config(server):
    return Configuration(f"{server.root}/wes")
