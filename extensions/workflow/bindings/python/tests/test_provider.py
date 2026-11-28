import json
import os
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

import pytest
from ga4gh_sdk import ExtensionError, ExtensionManager
from ga4gh_workflow import AcceptedRunError, Workflow, WorkflowError

EXAMPLE = Path(__file__).resolve().parents[5] / "examples/sequence-qc"
PIN = "45d41324353cb4900fdd999c307c2e36a3f2a410"
PINS = {"profile_revision": PIN, "vocabulary_revision": PIN}
IDENTITY = {"workflow": "public-qc@1"}


@pytest.fixture
def workflow(tmp_path):
    manager = ExtensionManager(str(tmp_path / "registry"), str(tmp_path / "artifacts"))
    with pytest.raises(ExtensionError):
        Workflow(manager)
    manager.install(os.environ["GA4GH_WORKFLOW_MANIFEST"])
    manager.enable("ga4gh-workflow-extension")
    return Workflow(manager)


def prepare(workflow, identity=IDENTITY):
    return workflow.prepare_file(EXAMPLE / "profile.json", PINS, json.loads((EXAMPLE / "request.json").read_bytes()), identity)


def test_bindings_return_identical_records(workflow):
    original = (EXAMPLE / "profile.json").read_bytes()
    with prepare(workflow) as plan:
        exported = plan.record
    with pytest.raises(WorkflowError) as invalid:
        workflow.prepare(original, PINS, json.loads((EXAMPLE / "invalid-request.json").read_bytes()), IDENTITY)
    outputs = {"metrics": {"sequence_count": 3, "total_length": 12, "mean_length": 4}, "report": {"class": "File", "location": "drs://example/report"}}
    projected = workflow.results(original, PINS, "run-1", "COMPLETE", outputs)
    evidence = workflow.verify(projected, "/report/location", b"report")
    report = workflow.compare(json.loads((EXAMPLE / "policy.json").read_bytes()), evidence, evidence)
    actual = {"plan": exported, "diagnostics": invalid.value.diagnostics, "record": evidence, "report": report}
    assert actual == json.loads(Path(os.environ["GA4GH_WORKFLOW_PARITY"]).read_bytes())


def test_prepared_bytes_submitted_once(workflow):
    wes = Mock()
    wes.run.return_value = SimpleNamespace(id="accepted-1")
    with prepare(workflow, dict(IDENTITY, descriptor_digest="sha256:" + "0" * 64)) as pinned:
        with pytest.raises(WorkflowError):
            pinned.submit(wes, {"main.cwl": b"wrong"})
    wes.run.assert_not_called()
    with prepare(workflow) as prepared:
        receipt = prepared.submit(wes, {"main.cwl": b"reviewed"})
        assert receipt["invocation"] == prepared.record
        assert receipt["run_id"] == "accepted-1"
        with pytest.raises(WorkflowError):
            prepared.submit(wes)
    wes.run.assert_called_once_with(json.loads((EXAMPLE / "request.json").read_bytes()), [("main.cwl", b"reviewed")])


@pytest.mark.parametrize("accepted", [True, False])
def test_completion_cannot_erase_wes_outcome(workflow, monkeypatch, accepted):
    wes = Mock()
    original = RuntimeError("ambiguous submission")
    if accepted:
        wes.run.return_value = SimpleNamespace(id="keep-this-id")
    else:
        wes.run.side_effect = original
    call = workflow._call

    def broken(operation, *args, **kwargs):
        if operation == "finish_submit":
            raise RuntimeError("PRIVATE plugin bytes")
        return call(operation, *args, **kwargs)

    monkeypatch.setattr(workflow, "_call", broken)
    with prepare(workflow) as prepared, pytest.raises(AcceptedRunError if accepted else RuntimeError) as failure:
        prepared.submit(wes)
    if accepted:
        assert failure.value.run_id == "keep-this-id"
        assert "PRIVATE" not in str(failure.value)
    else:
        assert failure.value is original
    wes.run.assert_called_once()
    with prepare(workflow):
        pass
