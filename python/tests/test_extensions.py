"""The Python API uses the Rust C ABI loader and owns native resources safely."""
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, HTTPServer
import gc
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import sysconfig
import time

import pytest
from ga4gh_sdk import ExtensionError, ExtensionManager

ROOT = Path(__file__).resolve().parents[2]


def compile_fixture(directory, name, version="1.0.0"):
    suffix = ".dll" if os.name == "nt" else ".dylib" if sys.platform == "darwin" else ".so"
    library = directory / f"{name}-{version}{suffix}"
    definitions = {
        "PLUGIN_NAME": name,
        "PLUGIN_VERSION": version,
        "TARGET_TRIPLE": ExtensionManager.host_target(),
    }
    compiler = shlex.split(os.environ.get("CC") or sysconfig.get_config_var("CC") or "cc")
    if os.name == "nt":
        command = ["cl", "/LD", f"/Fe:{library}", f"/I{ROOT / 'extension-abi/include'}"]
        command += [f"/D{k}={json.dumps(v)}" for k, v in definitions.items()]
    else:
        command = compiler + ["-dynamiclib" if sys.platform == "darwin" else "-shared", "-fPIC", "-o", str(library), "-I", str(ROOT / "extension-abi/include")]
        command += [f"-D{k}={json.dumps(v)}" for k, v in definitions.items()]
    subprocess.run(command + [str(ROOT / "lib/tests/fixtures/native-extension/extension.c")], cwd=directory, check=True, capture_output=True)
    return library


def install(manager, directory, name, library, version="1.0.0"):
    manifest = directory / f"{name}-{version}.json"
    digest = hashlib.sha512(library.read_bytes()).hexdigest()
    manifest.write_text(json.dumps({
        "manifest_version": 1, "name": name, "version": version,
        "target": manager.host_target(), "abi_version": 1, "digest": digest,
        "capabilities": {"test": [1, 2]}, "path": str(library),
    }))
    manager.install(str(manifest))
    return digest


def wait(path):
    deadline = time.monotonic() + 10
    while not path.exists():
        assert time.monotonic() < deadline, f"Timed out waiting for {path}"
        time.sleep(0.002)


def test_instances_handles_errors_and_restart(tmp_path):
    registry = tmp_path / "registry.json"
    root = tmp_path / "artifacts"
    manager = ExtensionManager(str(registry), str(root))
    assert not registry.exists()
    library = compile_fixture(tmp_path, "python-fixture")
    first_digest = install(manager, tmp_path, "python-fixture", library)
    manager.enable("python-fixture")
    resolved = manager.resolve("python-fixture", capabilities={"test": [1, 2, 3]})
    assert resolved.artifact()["digest"] == first_digest
    assert resolved.capabilities() == {"test": 2}
    module = manager.load(resolved)
    assert module.descriptor()["name"] == "python-fixture"
    first = manager.create_instance(resolved, b"first")
    second = manager.create_instance(resolved, b"second")
    assert first.invoke("test", "configuration", b"") == b"first"
    assert second.invoke("test", "configuration", b"") == b"second"
    with pytest.raises(ExtensionError) as failure:
        first.invoke("test", "fail", b"SECRET")
    assert failure.value.kind == "InvocationFailed"
    assert failure.value.status == 8
    assert failure.value.private_payload == b"SECRET"
    assert "SECRET" not in str(failure.value) + repr(failure.value)
    with pytest.raises(ExtensionError) as failure:
        manager.create_instance(resolved, b"fail SECRET_CONFIG")
    assert failure.value.kind == "InitializationFailed"
    assert "SECRET_CONFIG" not in str(failure.value) + repr(failure.value)
    dropped = tmp_path / "destroyed"
    first.invoke("test", "set_drop_path", str(dropped).encode())
    method = first.method("test", "echo")
    # GIL release permits both native calls and their progress markers to coexist.
    prefixes = [tmp_path / "one", tmp_path / "two"]
    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(instance.invoke, "test", "wait", str(prefix).encode()) for instance, prefix in zip([first, second], prefixes)]
        for prefix in prefixes:
            wait(prefix.with_suffix(".entered"))
        stale = ExtensionManager(str(registry), str(root))
        manager.disable("python-fixture")
        with pytest.raises(ExtensionError) as failure:
            stale.create_instance(resolved, b"late")
        assert failure.value.kind == "Disabled"
        for prefix in prefixes:
            prefix.with_suffix(".release").touch()
        for future in futures:
            future.result(timeout=10)
    manager.enable("python-fixture")
    new_library = compile_fixture(tmp_path, "python-fixture", "2.0.0")
    new_digest = install(manager, tmp_path, "python-fixture", new_library, "2.0.0")
    updated = manager.resolve("python-fixture", capabilities={"test": [1]})
    with pytest.raises(ExtensionError) as failure:
        manager.create_instance(updated, b"new")
    assert failure.value.kind == "RestartRequired"
    assert failure.value.current["digest"] == first_digest
    assert failure.value.requested["digest"] == new_digest
    manager.select("python-fixture", first_digest)
    assert manager.resolve("python-fixture").artifact()["digest"] == first_digest
    manager.remove("python-fixture")
    assert manager.list() == []
    assert any(root.rglob("library*"))
    with pytest.raises(ExtensionError) as failure:
        stale.create_instance(resolved, b"removed")
    assert failure.value.kind == "Missing"
    del first, second, module, manager, stale
    gc.collect()
    assert not dropped.exists()
    assert method.call(b"still live") == b"still live"
    del method
    gc.collect()
    assert dropped.read_text() == "destroyed"


def test_management_never_executes_library(tmp_path):
    manager = ExtensionManager(str(tmp_path / "registry.json"), str(tmp_path / "artifacts"))
    not_executable = tmp_path / "not-a-library"
    not_executable.write_bytes(b"not executable")
    digest = install(manager, tmp_path, "metadata-only", not_executable)
    manager.enable("metadata-only")
    manager.select("metadata-only", digest)
    assert manager.list()[0]["selected"] == digest
    manager.disable("metadata-only")
    manager.remove("metadata-only")
    assert manager.list() == []


def test_installation_sources_and_resource_errors_are_redacted(tmp_path):
    body = b"metadata only"

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            assert self.path == "/artifact?token=PRIVATE_TOKEN"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    registry = tmp_path / "registry.json"
    manager = ExtensionManager(str(registry), str(tmp_path / "artifacts"))
    manifest = tmp_path / "manifest.json"
    metadata = {
        "manifest_version": 1, "name": "python-source", "version": "1.0.0",
        "target": manager.host_target(), "abi_version": 1,
        "digest": hashlib.sha512(body).hexdigest(), "capabilities": {},
    }
    with HTTPServer(("127.0.0.1", 0), Handler) as server:
        metadata["path"] = f"http://127.0.0.1:{server.server_port}/artifact?token=PRIVATE_TOKEN"
        manifest.write_text(json.dumps(metadata))
        with ThreadPoolExecutor(max_workers=1) as pool:
            future = pool.submit(server.serve_forever)
            try:
                manager.install(str(manifest))
            finally:
                server.shutdown()
            future.result(timeout=5)
    installed = manager.list()
    assert "PRIVATE_TOKEN" not in json.dumps(installed) + registry.read_text()
    artifact = installed[0]["artifacts"][0]
    assert artifact["manifest"]["path"] == Path(artifact["library_path"]).name
    source = tmp_path / "too-large"
    with source.open("wb") as stream:
        stream.truncate(512 * 1024 * 1024 + 1)
    metadata["path"] = str(source)
    manifest.write_text(json.dumps(metadata))
    with pytest.raises(ExtensionError) as failure:
        manager.install(str(manifest))
    assert failure.value.kind == "ArtifactTooLarge"
    assert manager.list() == installed
