"""Extending the Python bindings: subclasses, full task JSON, extra endpoints."""

import base64
import json
import os
import subprocess
import sys
import urllib.request
from pathlib import Path

import pytest

import GA4GH
from GA4GH import Configuration, ListTasksParams, TES, TesTask, Transport
from examples.tes_extension import ProvenanceTES

from .mock_server import TASK


class PluginTransport(Transport):
    """A downstream transport with helpers for extra endpoints."""

    def update_plugin(self, plugin_id, data):
        return json.loads(self.put(f"plugins/{plugin_id}", json.dumps(data)))


@pytest.mark.parametrize(
    "cls",
    [
        GA4GH.Configuration,
        GA4GH.ServiceInfo,
        GA4GH.Transport,
        GA4GH.TES,
        GA4GH.Task,
        GA4GH.TesTask,
        GA4GH.ListTasksParams,
        GA4GH.ListTasksResponse,
    ],
)
def test_classes_can_be_subclassed(cls):
    subclass = type(f"My{cls.__name__}", (cls,), {})
    assert issubclass(subclass, cls)


def test_tes_subclass_calls_extra_endpoint(server, config):
    tes = ProvenanceTES(config)
    assert isinstance(tes, TES)
    assert tes.provenance("task-1") == {"events": ["created"]}
    assert server.last()["path"] == "/api/tasks/task-1/provenance"
    with pytest.raises(RuntimeError, match="404"):
        tes.provenance("task/other")
    assert server.last()["path"] == "/api/tasks/task%2Fother/provenance"


def test_checked_in_examples_run_against_mock(server):
    python_dir = Path(__file__).resolve().parents[1]
    env = {**os.environ, "GA4GH_TES_URL": server.base}
    task_example = subprocess.run(
        [sys.executable, "tests/example.py"],
        cwd=python_dir,
        env=env,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert task_example.returncode == 0, task_example.stderr
    assert "TASK ID: task-1" in task_example.stdout
    assert "STATE:   COMPLETE" in task_example.stdout
    with urllib.request.urlopen(f"{server.root}/__requests__", timeout=10) as response:
        requests = json.load(response)
    submitted = [
        request for request in requests
        if request["method"] == "POST" and request["path"] == "/api/tasks"
    ][-1]
    assert json.loads(base64.b64decode(submitted["body"])) == {
        "name": "hello",
        "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
    }

    extension_example = subprocess.run(
        [sys.executable, "examples/tes_extension.py", server.base, "task-1"],
        cwd=python_dir,
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert extension_example.returncode == 0, extension_example.stderr
    assert json.loads(extension_example.stdout) == {"events": ["created"]}
    assert server.last()["path"] == "/api/tasks/task-1/provenance"


def test_transport_subclass_calls_extra_endpoint(server, config):
    transport = PluginTransport(config)
    assert transport.update_plugin("p1", {"enabled": True}) == {"updated": True}
    request = server.last()
    assert request["method"] == "PUT"
    assert json.loads(request["body"]) == {"enabled": True}


def test_task_keeps_full_json_including_unknown_fields(server, config):
    task = TES(config).get("FULL", "task-1")
    assert isinstance(task, TesTask)
    assert task.id == "task-1"
    assert task.status == "COMPLETE"
    assert task.to_dict() == TASK
    assert json.loads(task.json()) == TASK
    assert server.last()["query"] == {"view": ["FULL"]}


def test_listed_tasks_keep_full_json(server, config):
    response = TES(config).list()
    assert response.next_page_token == "next"
    assert [task.to_dict() for task in response.tasks] == [TASK]


def test_task_cancel_and_status_still_work(server, config):
    task = GA4GH.Task("task-1", Transport(config))
    assert task.status() is GA4GH.TesState.COMPLETE
    assert task.cancel() is None
    assert server.last()["path"] == "/api/tasks/task-1:cancel"


def test_query_parameters_on_every_method(server, config):
    transport = Transport(config)
    params = {"force": True, "tag": ["a", "b"], "n": 3, "skip": None}
    expected = {"force": ["true"], "tag": ["a", "b"], "n": ["3"]}
    transport.get("tasks/task-1", params)
    assert server.last()["query"] == expected
    transport.post("runs", '{"x": 1}', params)
    assert server.last()["query"] == expected
    assert json.loads(server.last()["body"]) == {"x": 1}
    transport.put("plugins/p1", "{}", params=params)
    assert server.last()["query"] == expected
    transport.delete("plugins/p1", params=params)
    assert server.last()["query"] == expected


def test_query_parameters_as_json_string_and_dict_data(server, config):
    transport = Transport(config)
    transport.get("tasks/task-1", '{"view": "BASIC"}')
    assert server.last()["query"] == {"view": ["BASIC"]}
    transport.post("runs", {"a": [1, "b"]})
    assert json.loads(server.last()["body"]) == {"a": [1, "b"]}


def test_query_parameters_are_encoded_and_appended(server, config):
    transport = Transport(config)
    transport.get("tasks/task-1?view=MINIMAL", {"name": "a b&c=d"})
    assert server.last()["query"] == {"view": ["MINIMAL"], "name": ["a b&c=d"]}
    transport.get("tasks/task-1?", {"view": "BASIC", "skip": [None]})
    assert server.last()["query"] == {"view": ["BASIC"]}
    transport.get("tasks/task-1#top", {"view": "FULL"})
    assert server.last()["query"] == {"view": ["FULL"]}
    transport.get("tasks/task-1", {})
    assert server.last()["query"] == {}


@pytest.mark.parametrize("params", ['["a"]', {"nested": {"a": 1}}, {"list": [{"a": 1}]}])
def test_invalid_query_parameters(config, params):
    with pytest.raises(ValueError, match="params"):
        Transport(config).get("tasks", params)


def test_multipart(server, config):
    transport = Transport(config)
    response = transport.post_multipart(
        "runs",
        fields=[("workflow_type", "CWL"), ("workflow_params", '{"a": 1}')],
        files=[
            ("workflow_attachment", "main.cwl", b"cwlVersion: v1.2\n"),
            ("workflow_attachment", "data.bin", b"\x00\x01", "application/octet-stream"),
        ],
        params={"dry": True},
    )
    assert json.loads(response) == {"run_id": "run-1"}
    request = server.last()
    assert request["query"] == {"dry": ["true"]}
    headers = {key.lower(): value for key, value in request["headers"].items()}
    content_type = headers["content-type"]
    assert content_type.startswith("multipart/form-data; boundary=")
    body = request["body"]
    assert b'name="workflow_type"\r\n\r\nCWL\r\n' in body
    assert b'name="workflow_attachment"; filename="main.cwl"' in body
    assert b"cwlVersion: v1.2\n" in body
    assert b"Content-Type: application/octet-stream\r\n\r\n\x00\x01" in body


@pytest.mark.parametrize(
    "files",
    [
        [("only-name",)],
        [("name", "file.txt", "text, not bytes")],
        [("name", "file.txt", b"x", "not a content type")],
    ],
)
def test_multipart_rejects_malformed_files(config, files):
    with pytest.raises(ValueError, match="files"):
        Transport(config).post_multipart("runs", files=files)


def test_multipart_wrong_argument_types_raise_type_error(config):
    with pytest.raises(TypeError):
        Transport(config).post_multipart("runs", fields={"workflow_type": "CWL"})


def base_args(cls, server, config):
    """The arguments that each class's constructor requires."""
    if cls is Configuration:
        return (server.base,)
    if cls is GA4GH.Task:
        return ("task-1", Transport(config))
    if cls is TesTask:
        return (TASK,)
    return (config,)


CONSTRUCTED_CLASSES = [Configuration, GA4GH.ServiceInfo, Transport, TES, GA4GH.Task, TesTask]


@pytest.mark.parametrize("cls", CONSTRUCTED_CLASSES)
def test_subclass_init_can_take_further_arguments(server, config, cls):
    class Extended(cls):
        def __init__(self, *args, token, **kwargs):
            super().__init__()  # __new__ has already set up the base class
            self.token = token

    instance = Extended(*base_args(cls, server, config), "extra", token="t")
    assert isinstance(instance, cls)
    assert instance.token == "t"


@pytest.mark.parametrize("cls", CONSTRUCTED_CLASSES)
def test_unexpected_arguments_raise_type_error(server, config, cls):
    plain_subclass = type(f"Plain{cls.__name__}", (cls,), {})
    for target in (cls, plain_subclass):
        with pytest.raises(TypeError, match="unexpected"):
            target(*base_args(cls, server, config), "extra")
        with pytest.raises(TypeError, match="unexpected"):
            target(*base_args(cls, server, config), extra=1)


def test_subclass_keeps_base_state(server):
    class NamedConfiguration(Configuration):
        def __init__(self, base_path, name):
            super().__init__()
            self.name = name

    named = NamedConfiguration(server.base, "cfg")
    assert (named.name, named.get_base_path()) == ("cfg", server.base)


def test_client_getters(server, config):
    tes = TES(config)
    assert tes.config.get_base_path() == server.base
    task = GA4GH.Task("task-1", tes.transport)
    assert json.loads(task.transport.get("tasks/task-1/provenance")) == {"events": ["created"]}


def test_http_errors_raise_runtime_error(config):
    with pytest.raises(RuntimeError, match="404"):
        Transport(config).get("missing")


def test_invalid_task_raises_runtime_error(config):
    with pytest.raises(RuntimeError, match="Invalid TES response"):
        TES(config).get("FULL", "bad")


def test_data_not_serialisable_raises_value_error(config):
    with pytest.raises(ValueError, match="not JSON-serialisable"):
        Transport(config).post("runs", object())


def test_data_str_is_parsed_as_json(config):
    with pytest.raises(ValueError, match="Invalid data JSON"):
        Transport(config).post("runs", "plain text is parsed as JSON")


def test_query_appended_after_trailing_ampersand(server, config):
    Transport(config).get("tasks/task-1?view=BASIC&", {"name": "n"})
    assert server.last()["query"] == {"view": ["BASIC"], "name": ["n"]}


def test_list_tasks_params(server, config):
    params = ListTasksParams(
        name_prefix="hel",
        state="complete",
        tag_key=["foo1", "foo2"],
        tag_value=["bar1", ""],
        page_size=10,
        page_token="tok",
        view="BASIC",
    )
    assert (params.name_prefix, params.state, params.page_size) == ("hel", "COMPLETE", 10)
    assert (params.tag_key, params.tag_value) == (["foo1", "foo2"], ["bar1", ""])
    assert (params.page_token, params.view) == ("tok", "BASIC")
    TES(config).list(params)
    assert server.last()["query"] == {
        "name_prefix": ["hel"],
        "state": ["COMPLETE"],
        "tag_key": ["foo1", "foo2"],
        "tag_value": ["bar1", ""],
        "page_size": ["10"],
        "page_token": ["tok"],
        "view": ["BASIC"],
    }
    assert ListTasksParams().view is None


def test_list_tasks_params_rejects_unknown_state():
    with pytest.raises(ValueError, match="state"):
        ListTasksParams(state="NOT_A_STATE")


def test_old_task_class_name_still_works():
    assert GA4GH.PyTesTask is GA4GH.TesTask
