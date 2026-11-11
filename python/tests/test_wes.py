"""The WES bindings: run requests from dicts and objects, attachments, typed results."""

import base64
import copy
import enum
import json
import os
import pickle
import subprocess
import sys
import urllib.request
from pathlib import Path
from types import MappingProxyType

import pytest

import GA4GH
from GA4GH import (
    Configuration,
    Run,
    RunListResponse,
    RunLog,
    RunRequest,
    RunSummary,
    ServiceType,
    TaskListResponse,
    TaskLog,
    Transport,
    WES,
    WesState,
)

from .mock_server import BAD_ACKNOWLEDGEMENTS, RUN_LOG, RUN_REQUEST, RUN_SUMMARY, TASK_LOG

STATES = [
    "UNKNOWN",
    "QUEUED",
    "INITIALIZING",
    "RUNNING",
    "PAUSED",
    "COMPLETE",
    "EXECUTOR_ERROR",
    "SYSTEM_ERROR",
    "CANCELED",
    "CANCELING",
    "PREEMPTED",
]

NEW_RUN = {
    "workflow_type": "CWL",
    "workflow_type_version": "v1.2",
    "workflow_url": "main.cwl",
    "workflow_params": {"message": "hello"},
    "tags": {"project": "demo"},
    "workflow_engine": "cwltool",
    "workflow_engine_version": "3.1",
    "workflow_engine_parameters": {"--debug": "true"},
}


def form_fields(request):
    """Parses a recorded multipart request into `(name, filename, content)` parts."""
    headers = {key.lower(): value for key, value in request["headers"].items()}
    boundary = headers["content-type"].split("boundary=", 1)[1].encode()
    parts = []
    for chunk in request["body"].split(b"--" + boundary)[1:-1]:
        head, _, content = chunk[2:-2].partition(b"\r\n\r\n")  # Each part is in CRLFs.
        disposition = head.split(b"\r\n")[0].decode()
        name = disposition.split('name="', 1)[1].split('"', 1)[0]
        filename = None
        if 'filename="' in disposition:
            filename = disposition.split('filename="', 1)[1].split('"', 1)[0]
        parts.append((name, filename, content))
    return parts


def text_fields(request):
    """The text parts of a recorded multipart request, by name."""
    return {name: content.decode() for name, filename, content in form_fields(request) if filename is None}


def attachments(request):
    """The file parts of a recorded multipart request, as `(filename, content)`."""
    return [(filename, content) for name, filename, content in form_fields(request) if filename is not None]


# States


def test_state_enum_has_all_wes_states():
    assert issubclass(WesState, enum.Enum)
    assert [state.name for state in WesState] == STATES
    assert [state.value for state in WesState] == STATES


def test_state_enum_members_are_strings():
    assert isinstance(WesState.RUNNING, str)
    assert WesState.RUNNING == "RUNNING"
    assert WesState("RUNNING") is WesState.RUNNING
    assert str(WesState.RUNNING) == "RUNNING"
    assert f"{WesState.RUNNING:<9}|" == "RUNNING  |"
    assert json.dumps({"state": WesState.RUNNING}) == '{"state": "RUNNING"}'
    assert WesState.__module__ == "GA4GH"
    assert pickle.loads(pickle.dumps(WesState.QUEUED)) is WesState.QUEUED
    assert WesState is not GA4GH.TesState


def test_service_type_has_wes():
    assert ServiceType.WES != ServiceType.TES


@pytest.mark.skipif(os.name != "posix", reason="HOME controls the config path on Unix")
def test_wes_section_loads_into_client(server, tmp_path, monkeypatch):
    config_dir = tmp_path / ".ga4gh-cli"
    config_dir.mkdir()
    (config_dir / "config.json").write_text(
        json.dumps(
            {
                "TES": {"base_path": "https://wrong.example/tes"},
                "WES": {"base_path": f"{server.root}/wes"},
            }
        )
    )
    monkeypatch.setenv("HOME", str(tmp_path))
    config = Configuration("https://example.org")
    config.from_file(ServiceType.WES)
    assert config.get_base_path() == f"{server.root}/wes"
    assert WES(config).list().runs[0].run_id == "run-1"


# Client


def test_client_checks_service_type(server, config):
    with pytest.raises(RuntimeError, match="not an instance of WES"):
        WES(config)


def test_client_hints_at_the_api_root_on_404(server):
    with pytest.raises(RuntimeError, match="Check that base_path is the WES API root"):
        WES(Configuration(f"{server.root}/missing"))


def test_client_reports_unreachable_service():
    with pytest.raises(RuntimeError, match="Failed to create WES instance"):
        WES(Configuration("http://127.0.0.1:1/wes"))


def requests_received(server):
    """All requests that the mock server received so far."""
    with urllib.request.urlopen(f"{server.root}/__requests__", timeout=10) as response:
        return json.load(response)


@pytest.mark.parametrize("root", sorted(BAD_ACKNOWLEDGEMENTS))
def test_failed_or_malformed_acknowledgements_raise_once(server, root):
    wes = WES(Configuration(f"{server.root}/{root}"))
    before = len(requests_received(server))
    with pytest.raises(RuntimeError, match="Failed to run workflow"):
        wes.run(RUN_REQUEST)
    posts = [(r["method"], r["path"]) for r in requests_received(server)[before:]]
    assert posts == [("POST", f"/{root}/runs")]


def test_client_getters(server, wes_config):
    wes = WES(wes_config)
    assert wes.config.get_base_path() == f"{server.root}/wes"
    assert isinstance(wes.transport, Transport)
    assert json.loads(wes.transport.get("runs/run-1/status"))["state"] == "COMPLETE"


# Running workflows


def test_run_from_dict(server, wes_config):
    run = WES(wes_config).run(NEW_RUN)
    assert isinstance(run, Run)
    assert run.id == "run-1"
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/wes/runs")
    fields = text_fields(request)
    assert fields["workflow_type"] == "CWL"
    assert fields["workflow_type_version"] == "v1.2"
    assert fields["workflow_url"] == "main.cwl"
    assert json.loads(fields["workflow_params"]) == NEW_RUN["workflow_params"]
    assert json.loads(fields["tags"]) == NEW_RUN["tags"]
    assert fields["workflow_engine"] == "cwltool"
    assert fields["workflow_engine_version"] == "3.1"
    assert json.loads(fields["workflow_engine_parameters"]) == NEW_RUN["workflow_engine_parameters"]
    assert attachments(request) == []


def test_run_from_request_object_and_other_mapping(server, wes_config):
    wes = WES(wes_config)
    for request in (RunRequest(RUN_REQUEST), MappingProxyType(RUN_REQUEST)):
        wes.run(request)
        fields = text_fields(server.last())
        assert set(fields) == {"workflow_type", "workflow_type_version", "workflow_url", "workflow_params", "tags"}


def test_run_from_file_path(server, wes_config, tmp_path):
    path = tmp_path / "run.json"
    path.write_text(json.dumps(RUN_REQUEST))
    for argument in (path, str(path)):
        WES(wes_config).run(argument)
        assert text_fields(server.last())["workflow_url"] == "main.cwl"


def test_run_takes_str_as_path_not_json_text(wes_config):
    with pytest.raises(ValueError, match="from_json"):
        WES(wes_config).run(json.dumps(RUN_REQUEST))


def test_run_with_attachments(server, wes_config, tmp_path):
    (tmp_path / "inputs.json").write_bytes(b'{"x": 1}')
    wes = WES(wes_config)
    wes.run(RUN_REQUEST, {"main.cwl": b"cwlVersion: v1.2\n", "tools/echo.cwl": bytearray(b"\x00\x01")})
    assert attachments(server.last()) == [
        ("main.cwl", b"cwlVersion: v1.2\n"),
        ("tools/echo.cwl", b"\x00\x01"),
    ]
    wes.run(RUN_REQUEST, [("main.cwl", b"a"), ("inputs.json", tmp_path / "inputs.json")])
    assert attachments(server.last()) == [("main.cwl", b"a"), ("inputs.json", b'{"x": 1}')]


@pytest.mark.parametrize(
    ("files", "message"),
    [
        ({"../main.cwl": b"x"}, "safe relative path"),
        ({"/main.cwl": b"x"}, "safe relative path"),
        ({"": b"x"}, "safe relative path"),
        ([("main.cwl", b"x"), ("main.cwl", b"y")], "duplicate"),
        ({"main.cwl": "text, not bytes"}, "bytes"),
        ([("main.cwl",)], "pairs"),
        (["main.cwl"], "pairs"),
    ],
)
def test_run_rejects_invalid_attachments_before_request(server, wes_config, files, message):
    wes = WES(wes_config)
    before = server.last()
    with pytest.raises(ValueError, match=message):
        wes.run(RUN_REQUEST, files)
    assert server.last() == before


def test_run_with_missing_attachment_file_raises_os_error(server, wes_config, tmp_path):
    wes = WES(wes_config)
    before = server.last()
    with pytest.raises(FileNotFoundError):
        wes.run(RUN_REQUEST, {"main.cwl": tmp_path / "missing.cwl"})
    with pytest.raises(ValueError, match="not a file"):
        wes.run(RUN_REQUEST, {"main.cwl": tmp_path})
    if hasattr(os, "mkfifo"):
        # Opening a FIFO without a writer would block; it must be rejected first.
        os.mkfifo(tmp_path / "pipe")
        with pytest.raises(ValueError, match="not a file"):
            wes.run(RUN_REQUEST, {"main.cwl": tmp_path / "pipe"})
    assert server.last() == before


@pytest.mark.parametrize(
    ("request_", "message"),
    [
        ({key: value for key, value in RUN_REQUEST.items() if key != "workflow_url"}, "workflow_url"),
        ({**RUN_REQUEST, "workflow_type": ""}, "workflow_type"),
        ({key: value for key, value in RUN_REQUEST.items() if key != "workflow_params"}, "workflow_params"),
        ({**RUN_REQUEST, "workflow_params": [1]}, "workflow_params must be a JSON object"),
        ({**RUN_REQUEST, "tags": {"n": 1}}, "run request"),
        ([RUN_REQUEST], "run request"),
        (object(), "run request"),
    ],
)
def test_run_rejects_invalid_requests_before_request(server, wes_config, request_, message):
    wes = WES(wes_config)
    before = server.last()
    with pytest.raises(ValueError, match=message):
        wes.run(request_)
    assert server.last() == before


# Runs


def test_run_status_returns_state_enum(server, wes_config):
    assert Run("run-1", Transport(wes_config)).status() is WesState.COMPLETE
    assert server.last()["path"] == "/wes/runs/run-1/status"


def test_run_cancel_returns_none(server, wes_config):
    assert Run("run-1", Transport(wes_config)).cancel() is None
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/wes/runs/run-1/cancel")


def test_run_ids_are_encoded(server, wes_config):
    with pytest.raises(RuntimeError, match="404"):
        Run("run/other", Transport(wes_config)).status()
    assert server.last()["path"] == "/wes/runs/run%2Fother/status"


def test_run_getters(wes_config):
    transport = Transport(wes_config)
    run = Run("run-1", transport)
    assert run.id == "run-1"
    assert json.loads(run.transport.get("runs/run-1/status"))["run_id"] == "run-1"


def check_run_log(log):
    assert isinstance(log, RunLog)
    assert log.run_id == "run-1"
    assert log.state is WesState.COMPLETE
    assert isinstance(log.request, RunRequest)
    assert log.request.to_dict() == RUN_REQUEST
    assert log.run_log == RUN_LOG["run_log"]
    assert log.task_logs_url == "runs/run-1/tasks"
    assert log.task_logs is None
    assert log.outputs == {"out": "hello"}
    assert log.to_dict() == RUN_LOG
    assert json.loads(log.json()) == RUN_LOG
    assert repr(log) == "RunLog(run_id='run-1', state='COMPLETE')"


def test_run_log_returns_typed_log(server, wes_config):
    check_run_log(Run("run-1", Transport(wes_config)).log())
    assert server.last()["path"] == "/wes/runs/run-1"


def test_get_returns_typed_log(server, wes_config):
    check_run_log(WES(wes_config).get("run-1"))
    assert server.last()["path"] == "/wes/runs/run-1"


def test_get_rejects_empty_id(server, wes_config):
    wes = WES(wes_config)
    before = server.last()
    with pytest.raises(ValueError, match="run ID"):
        wes.get("")
    assert server.last() == before


@pytest.mark.parametrize(
    "call",
    [
        lambda wes, transport: wes.get("bad"),
        lambda wes, transport: Run("bad", transport).log(),
        lambda wes, transport: Run("bad", transport).tasks(),
        lambda wes, transport: Run("bad", transport).task("t"),
    ],
)
def test_invalid_responses_raise_runtime_error(wes_config, call):
    with pytest.raises(RuntimeError, match="Invalid WES response"):
        call(WES(wes_config), Transport(wes_config))


def test_unknown_run_state_raises_runtime_error(wes_config):
    with pytest.raises(RuntimeError, match="run status"):
        Run("bad", Transport(wes_config)).status()


def test_invalid_run_list_raises_runtime_error(server):
    with pytest.raises(RuntimeError, match="Invalid WES response"):
        WES(Configuration(f"{server.root}/wes-bad-list")).list()


def test_run_log_without_state_is_unknown(wes_config):
    log = WES(wes_config).get("no-state")
    assert log.state is WesState.UNKNOWN
    assert (log.request, log.run_log, log.outputs) == (None, None, None)
    assert log.to_dict() == {"run_id": "no-state"}


# Listing


def test_list_returns_typed_response(server, wes_config):
    response = WES(wes_config).list()
    assert isinstance(response, RunListResponse)
    assert server.last()["query"] == {}
    [summary] = response.runs
    assert isinstance(summary, RunSummary)
    assert summary.run_id == "run-1"
    assert summary.state is WesState.COMPLETE
    assert summary.start_time == "2026-01-01T00:00:00Z"
    assert summary.end_time is None
    assert summary.tags == {"project": "demo"}
    assert summary.to_dict() == RUN_SUMMARY
    assert json.loads(summary.json()) == RUN_SUMMARY
    assert repr(summary) == "RunSummary(run_id='run-1', state='COMPLETE')"
    assert response.next_page_token == "next"
    assert response.to_dict() == {"runs": [RUN_SUMMARY], "next_page_token": "next"}
    assert json.loads(response.json()) == response.to_dict()


def test_list_sends_paging_parameters(server, wes_config):
    WES(wes_config).list(page_size=10, page_token="tok")
    assert server.last()["query"] == {"page_size": ["10"], "page_token": ["tok"]}


def test_list_accepts_bare_arrays(server):
    response = WES(Configuration(f"{server.root}/starter-kit")).list()
    assert [summary.to_dict() for summary in response.runs] == [RUN_SUMMARY]
    assert response.next_page_token is None
    assert response.to_dict() == {"runs": [RUN_SUMMARY]}


def test_run_tasks_return_typed_task_logs(server, wes_config):
    run = Run("run-1", Transport(wes_config))
    response = run.tasks(page_size=5, page_token="tok")
    assert isinstance(response, TaskListResponse)
    assert server.last()["query"] == {"page_size": ["5"], "page_token": ["tok"]}
    assert response.next_page_token == "next"
    assert response.to_dict() == {"task_logs": [TASK_LOG], "next_page_token": "next"}
    assert json.loads(response.json()) == response.to_dict()
    [task] = response.task_logs
    for task in (task, run.task("task-1")):
        assert isinstance(task, TaskLog)
        assert (task.id, task.name, task.cmd) == ("task-1", "echo", ["echo", "hello"])
        assert (task.exit_code, task.tes_uri) == (0, "https://tes.example/v1/tasks/1")
        assert (task.start_time, task.end_time, task.stdout, task.stderr, task.system_logs) == (None,) * 5
        assert task.to_dict() == TASK_LOG
        assert json.loads(task.json()) == TASK_LOG
        assert repr(task) == "TaskLog(id='task-1', name='echo')"
    assert server.last()["path"] == "/wes/runs/run-1/tasks/task-1"


@pytest.mark.parametrize("run_id", ["", ".", ".."])
def test_run_rejects_ids_that_are_no_path_segment(server, wes_config, run_id):
    wes = WES(wes_config)
    before = server.last()
    with pytest.raises(ValueError, match="run ID"):
        Run(run_id, Transport(wes_config))
    with pytest.raises(ValueError, match="run ID"):
        wes.get(run_id)
    with pytest.raises(ValueError, match="task ID"):
        Run("run-1", Transport(wes_config)).task(run_id)
    assert server.last() == before


@pytest.mark.skipif(os.name != "posix" or os.geteuid() == 0, reason="needs file permissions")
def test_run_with_unreadable_attachment_raises_permission_error(server, wes_config, tmp_path):
    path = tmp_path / "main.cwl"
    path.write_bytes(b"x")
    path.chmod(0)
    wes = WES(wes_config)
    before = server.last()
    with pytest.raises(PermissionError) as error:
        wes.run(RUN_REQUEST, {"main.cwl": path})
    assert error.value.filename == str(path)
    assert server.last() == before


def test_run_task_rejects_empty_id(wes_config):
    with pytest.raises(ValueError, match="task ID"):
        Run("run-1", Transport(wes_config)).task("")


@pytest.mark.parametrize(
    "call",
    [
        lambda wes, transport: wes.get("missing"),
        lambda wes, transport: Run("missing", transport).status(),
        lambda wes, transport: Run("missing", transport).cancel(),
        lambda wes, transport: Run("missing", transport).log(),
        lambda wes, transport: Run("missing", transport).tasks(),
        lambda wes, transport: Run("missing", transport).task("t"),
    ],
)
def test_failed_requests_raise_runtime_error(wes_config, call):
    with pytest.raises(RuntimeError):
        call(WES(wes_config), Transport(wes_config))


# Run requests


def test_request_constructors():
    text = json.dumps(NEW_RUN)
    for request in (
        RunRequest(NEW_RUN),
        RunRequest(text),
        RunRequest.from_dict(NEW_RUN),
        RunRequest.from_json(text),
        RunRequest(RunRequest(NEW_RUN)),
    ):
        assert request.to_dict() == NEW_RUN


def test_request_from_file(tmp_path):
    path = tmp_path / "run.json"
    path.write_text(json.dumps(NEW_RUN))
    assert RunRequest.from_file(path).to_dict() == NEW_RUN
    assert RunRequest.from_file(str(path)).to_dict() == NEW_RUN
    with pytest.raises(FileNotFoundError):
        RunRequest.from_file(tmp_path / "missing.json")


def test_request_fields():
    request = RunRequest({**NEW_RUN, "extension_field": [1]})
    assert request.workflow_type == "CWL"
    assert request.workflow_type_version == "v1.2"
    assert request.workflow_url == "main.cwl"
    assert request.workflow_params == {"message": "hello"}
    assert request.tags == {"project": "demo"}
    assert request.workflow_engine == "cwltool"
    assert request.workflow_engine_version == "3.1"
    assert request.workflow_engine_parameters == {"--debug": "true"}
    assert request.to_dict()["extension_field"] == [1]
    assert repr(request) == "RunRequest(workflow_type='CWL', workflow_url='main.cwl')"
    minimal = RunRequest({key: NEW_RUN[key] for key in ("workflow_type", "workflow_type_version", "workflow_url")})
    assert (minimal.workflow_params, minimal.tags, minimal.workflow_engine) == (None, None, None)


def test_request_to_dict_returns_a_copy():
    request = RunRequest(NEW_RUN)
    request.to_dict()["workflow_params"]["message"] = "changed"
    assert request.workflow_params == {"message": "hello"}


@pytest.mark.parametrize("request_", ["[]", "{", {"workflow_type": "CWL"}, ["a"], {**RUN_REQUEST, "workflow_params": "wrong"}])
def test_request_constructor_rejects_invalid_requests(request_):
    with pytest.raises(ValueError, match="run request"):
        RunRequest(request_)


class PicklableRequest(RunRequest):
    """A subclass at module level, so that pickle can find it."""


def test_request_can_be_pickled_and_copied():
    for request in (RunRequest(NEW_RUN), PicklableRequest(NEW_RUN)):
        for clone in (pickle.loads(pickle.dumps(request)), copy.copy(request), copy.deepcopy(request)):
            assert type(clone) is type(request)
            assert clone.to_dict() == NEW_RUN


def test_copies_keep_subclass_attributes():
    request = PicklableRequest(NEW_RUN)
    request.note = "kept"
    for clone in (pickle.loads(pickle.dumps(request)), copy.copy(request), copy.deepcopy(request)):
        assert clone.note == "kept"


def test_request_classmethods_return_subclass():
    class MyRequest(RunRequest):
        pass

    assert type(MyRequest.from_dict(NEW_RUN)) is MyRequest
    assert type(MyRequest.from_json(json.dumps(NEW_RUN))) is MyRequest


# Extending


WES_CLASSES = [WES, Run, RunRequest, RunLog, RunSummary, RunListResponse, TaskLog, TaskListResponse]


@pytest.mark.parametrize("cls", WES_CLASSES)
def test_classes_can_be_subclassed(cls):
    subclass = type(f"My{cls.__name__}", (cls,), {})
    assert issubclass(subclass, cls)


def base_args(cls, wes_config):
    """The arguments that each constructible class requires."""
    if cls is Run:
        return ("run-1", Transport(wes_config))
    if cls is RunRequest:
        return (RUN_REQUEST,)
    return (wes_config,)


@pytest.mark.parametrize("cls", [WES, Run, RunRequest])
def test_subclass_init_can_take_further_arguments(wes_config, cls):
    class Extended(cls):
        def __init__(self, *args, token, **kwargs):
            super().__init__()  # __new__ has already set up the base class
            self.token = token

    instance = Extended(*base_args(cls, wes_config), "extra", token="t")
    assert isinstance(instance, cls)
    assert instance.token == "t"


@pytest.mark.parametrize("cls", [WES, Run, RunRequest])
def test_unexpected_arguments_raise_type_error(wes_config, cls):
    plain_subclass = type(f"Plain{cls.__name__}", (cls,), {})
    for target in (cls, plain_subclass):
        with pytest.raises(TypeError, match="unexpected"):
            target(*base_args(cls, wes_config), "extra")


def test_wes_subclass_calls_extra_endpoint(server, wes_config):
    class ProvenanceWES(WES):
        def provenance(self, run_id):
            return json.loads(self.transport.get(f"runs/{run_id}/status"))

    assert ProvenanceWES(wes_config).provenance("run-1")["state"] == "COMPLETE"
    assert server.last()["path"] == "/wes/runs/run-1/status"


def test_checked_in_example_runs_against_mock(server, tmp_path):
    workflow = tmp_path / "hello.cwl"
    workflow.write_bytes(b"cwlVersion: v1.2\nclass: CommandLineTool\n")
    example = subprocess.run(
        [sys.executable, "examples/wes_run.py", f"{server.root}/wes", str(workflow)],
        cwd=Path(__file__).resolve().parents[1],
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert example.returncode == 0, example.stderr
    assert "RUN ID: run-1" in example.stdout
    assert "STATE:  COMPLETE" in example.stdout
    assert json.loads(example.stdout.split("COMPLETE\n", 1)[1]) == RUN_LOG
    with urllib.request.urlopen(f"{server.root}/__requests__", timeout=10) as response:
        requests = json.load(response)
    [submitted] = [
        request for request in requests[-4:] if (request["method"], request["path"]) == ("POST", "/wes/runs")
    ]
    submitted["body"] = base64.b64decode(submitted["body"])
    assert text_fields(submitted)["workflow_url"] == "hello.cwl"
    assert attachments(submitted) == [("hello.cwl", workflow.read_bytes())]
