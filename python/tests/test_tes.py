"""The TES bindings: tasks from dicts and objects, typed results and the state enum."""

import copy
import enum
import errno
import json
import os
import pickle
import sys
from pathlib import Path
from types import MappingProxyType

import pytest

import GA4GH
from GA4GH import (
    ListTasksParams,
    ListTasksResponse,
    Task,
    TES,
    TesState,
    TesTask,
    Transport,
)

from .mock_server import TASK

NEW_TASK = {
    "name": "hello",
    "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
    "tags": {"project": "demo"},
    "extension_field": {"kept": True},
}

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
    "PREEMPTED",
    "CANCELING",
]


def test_state_enum_has_all_tes_states():
    assert issubclass(TesState, enum.Enum)
    assert [state.name for state in TesState] == STATES
    assert [state.value for state in TesState] == STATES


def test_state_enum_members_are_strings():
    assert isinstance(TesState.RUNNING, str)
    assert TesState.RUNNING == "RUNNING"
    assert TesState("RUNNING") is TesState.RUNNING
    assert str(TesState.RUNNING) == "RUNNING"
    assert f"{TesState.RUNNING:<9}|" == "RUNNING  |"
    assert json.dumps({"state": TesState.RUNNING}) == '{"state": "RUNNING"}'
    assert TesState.__module__ == "GA4GH"
    assert pickle.loads(pickle.dumps(TesState.QUEUED)) is TesState.QUEUED


def test_state_enum_leaves_the_package_alone():
    assert GA4GH.__file__.endswith("__init__.py")
    assert not hasattr(GA4GH, "enum")
    assert sys.modules["GA4GH"] is GA4GH


def test_state_enum_rejects_unknown_names():
    with pytest.raises(ValueError):
        TesState("NOT_A_STATE")


def test_create_from_dict(server, config):
    task = TES(config).create(NEW_TASK)
    assert isinstance(task, Task)
    assert task.id == "task-1"
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/api/tasks")
    assert json.loads(request["body"]) == NEW_TASK


def test_create_from_task_object(server, config):
    TES(config).create(TesTask.from_dict(NEW_TASK))
    assert json.loads(server.last()["body"]) == NEW_TASK


def test_create_from_file_path(server, config, tmp_path):
    path = tmp_path / "task.json"
    path.write_text(json.dumps(NEW_TASK))
    for argument in (path, str(path)):
        TES(config).create(argument)
        assert json.loads(server.last()["body"]) == NEW_TASK


def test_create_from_other_mapping(server, config):
    TES(config).create(MappingProxyType(NEW_TASK))
    assert json.loads(server.last()["body"]) == NEW_TASK


def test_create_takes_str_as_path_not_json_text(config):
    with pytest.raises(ValueError, match="from_json"):
        TES(config).create(json.dumps(NEW_TASK))


def test_create_takes_paths_with_surrogate_escapes(config, tmp_path):
    path = os.fsdecode(os.fsencode(str(tmp_path)) + b"/t\xff.json")
    with pytest.raises(FileNotFoundError):
        TES(config).create(path)


def test_create_takes_path_objects_that_start_with_a_brace(server, config, tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    Path("{dir}").mkdir()
    Path("{dir}/task.json").write_text(json.dumps(NEW_TASK))
    TES(config).create(Path("{dir}/task.json"))
    assert json.loads(server.last()["body"]) == NEW_TASK


def test_missing_file_has_system_message(tmp_path):
    with pytest.raises(FileNotFoundError) as error:
        TesTask.from_file(tmp_path / "missing.json")
    assert error.value.strerror == os.strerror(errno.ENOENT)
    assert error.value.filename == str(tmp_path / "missing.json")


def test_file_that_is_not_utf8_raises_value_error(tmp_path):
    path = tmp_path / "task.json"
    path.write_bytes(b"\xff\xfe{}")
    with pytest.raises(ValueError, match="UTF-8"):
        TesTask.from_file(path)


class PicklableTask(TesTask):
    """A subclass at module level, so that pickle can find it."""


def test_task_can_be_pickled_and_copied():
    for task in (TesTask(NEW_TASK), PicklableTask(NEW_TASK)):
        for clone in (pickle.loads(pickle.dumps(task)), copy.copy(task), copy.deepcopy(task)):
            assert type(clone) is type(task)
            assert clone.to_dict() == NEW_TASK


def test_copies_keep_subclass_attributes():
    task = PicklableTask(NEW_TASK)
    task.note = "kept"
    for clone in (pickle.loads(pickle.dumps(task)), copy.copy(task), copy.deepcopy(task)):
        assert clone.note == "kept"


def test_create_from_missing_file_raises_os_error(config, tmp_path):
    with pytest.raises(FileNotFoundError):
        TES(config).create(tmp_path / "missing.json")
    with pytest.raises(FileNotFoundError):
        TES(config).create(str(tmp_path / "missing.json"))


@pytest.mark.parametrize(
    "task",
    [
        [NEW_TASK],
        {"executors": "not a list"},
        {"executors": [{"command": ["true"]}]},
        object(),
    ],
)
def test_create_rejects_invalid_tasks(config, task):
    with pytest.raises(ValueError, match="task"):
        TES(config).create(task)


def test_task_status_returns_state_enum(server, config):
    status = Task("task-1", Transport(config)).status()
    assert status is TesState.COMPLETE
    assert server.last()["query"] == {"view": ["MINIMAL"]}


def test_task_cancel_returns_none(server, config):
    assert Task("task-1", Transport(config)).cancel() is None
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/api/tasks/task-1:cancel")


def test_get_returns_typed_task(config):
    task = TES(config).get("FULL", "task-1")
    assert isinstance(task, TesTask)
    assert task.id == "task-1"
    assert task.state is TesState.COMPLETE
    assert task.status is TesState.COMPLETE
    assert task.name == "hello"
    assert task.description is None
    assert task.executors == TASK["executors"]
    assert task.tags == TASK["tags"]
    assert task.inputs is None
    assert task.to_dict() == TASK
    assert repr(task) == "TesTask(id='task-1', state='COMPLETE')"


def test_states_that_the_spec_does_not_define_read_as_unknown(config):
    # TES defines UNKNOWN as the safe default; the JSON keeps the server's value.
    task = TES(config).get("FULL", "new-state")
    assert task.state is TesState.UNKNOWN
    assert task.to_dict() == {"id": "new-state", "state": "SUSPENDED"}


def test_task_fields_of_a_new_task():
    task = TesTask(NEW_TASK)
    assert task.id is None
    assert task.state is TesState.UNKNOWN
    assert task.name == "hello"
    assert task.tags == {"project": "demo"}
    assert task.to_dict() == NEW_TASK


def test_task_to_dict_returns_a_copy():
    task = TesTask(NEW_TASK)
    task.to_dict()["name"] = "changed"
    assert task.name == "hello"


def test_task_constructors():
    text = json.dumps(NEW_TASK)
    for task in (TesTask(NEW_TASK), TesTask(text), TesTask.from_dict(NEW_TASK), TesTask.from_json(text)):
        assert task.to_dict() == NEW_TASK
    assert TesTask(TesTask(NEW_TASK)).to_dict() == NEW_TASK


def test_task_from_file(tmp_path):
    path = tmp_path / "task.json"
    path.write_text(json.dumps(NEW_TASK))
    assert TesTask.from_file(path).to_dict() == NEW_TASK
    assert TesTask.from_file(str(path)).to_dict() == NEW_TASK
    with pytest.raises(FileNotFoundError):
        TesTask.from_file(tmp_path / "missing.json")


@pytest.mark.parametrize("task", ["[]", "{", {"executors": 1}, ["a"]])
def test_task_constructor_rejects_invalid_tasks(task):
    with pytest.raises(ValueError, match="task"):
        TesTask(task)


def test_task_classmethods_return_subclass():
    class MyTask(TesTask):
        pass

    assert type(MyTask.from_dict(NEW_TASK)) is MyTask
    assert type(MyTask.from_json(json.dumps(NEW_TASK))) is MyTask


def test_list_returns_typed_response(config):
    response = TES(config).list()
    assert isinstance(response, ListTasksResponse)
    assert [task.state for task in response.tasks] == [TesState.COMPLETE]
    assert response.next_page_token == "next"
    assert response.to_dict() == {"tasks": [TASK], "next_page_token": "next"}
    assert json.loads(response.json()) == response.to_dict()


def test_list_tasks_params_state_is_enum(server, config):
    params = ListTasksParams(state=TesState.RUNNING)
    assert params.state is TesState.RUNNING
    assert ListTasksParams(state="queued").state is TesState.QUEUED
    assert ListTasksParams().state is None
    TES(config).list(params)
    assert server.last()["query"] == {"state": ["RUNNING"]}


def test_list_tasks_params_rejects_unknown_state_in_any_case():
    with pytest.raises(ValueError, match="Invalid TES state"):
        ListTasksParams(state="running!")


def test_server_task_without_state_is_unknown(config):
    assert TesTask({"id": "x"}).state is TesState.UNKNOWN


@pytest.mark.parametrize(
    "call",
    [
        lambda tes, transport: tes.get("FULL", "missing"),
        lambda tes, transport: Task("missing", transport).status(),
        lambda tes, transport: Task("missing", transport).cancel(),
    ],
)
def test_failed_requests_raise_runtime_error(config, call):
    with pytest.raises(RuntimeError):
        call(TES(config), Transport(config))
