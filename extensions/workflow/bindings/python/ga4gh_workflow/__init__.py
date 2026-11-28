"""Revision-1 workflow client. Importing this module does not load a provider."""
from __future__ import annotations

import base64
import json
from pathlib import Path
from threading import Lock
from typing import TYPE_CHECKING, Any, Mapping, NotRequired, Self, TypedDict

if TYPE_CHECKING:
    from ga4gh_sdk import ExtensionInstance, ExtensionManager, WES

_CAP = "ga4gh.workflow"
_LIMIT = 64 << 20


class Pins(TypedDict):
    profile_revision: str
    vocabulary_revision: str


class Identity(TypedDict):
    workflow: str
    descriptor_digest: NotRequired[str | None]


class PlanRecord(TypedDict):
    fingerprint: str
    schema_digest: str
    request: dict[str, Any]
    pins: Pins
    identity: Identity


class ResultRecord(TypedDict):
    format_version: int
    run_reference: str
    execution_state: str
    schema_digest: str
    outputs: dict[str, Any]
    artifacts: dict[str, Any]
    context: dict[str, str]


class Checksum(TypedDict):
    algorithm: str
    digest: str
    evidence_source: str


class Policy(TypedDict):
    id: str
    version: int
    outputs: dict[str, Any]
    required_context: list[str]


class Report(TypedDict):
    policy: Policy
    outcome: str
    outputs: dict[str, str]


class Receipt(TypedDict):
    run_id: str
    invocation: PlanRecord


class Diagnostic(TypedDict):
    instance_location: str
    schema_location: str
    message: str


class WorkflowError(Exception):
    def __init__(self, code: str, message: str, diagnostics: list[Diagnostic]):
        super().__init__(message)
        self.code, self.diagnostics = code, diagnostics


class ProtocolError(Exception):
    """The native reply did not match the negotiated interface."""


class AcceptedRunError(Exception):
    def __init__(self, run_id: str):
        super().__init__(f"WES accepted run {run_id!r}; completion failed, do not resubmit")
        self.run_id = run_id


class Workflow:
    def __init__(self, manager: ExtensionManager):
        resolved = manager.resolve("ga4gh-workflow-extension", capabilities={_CAP: [1]})
        self._instance: ExtensionInstance = manager.create_instance(resolved, b"{}")

    def _call(self, operation: str, request: dict, field: str = "ok") -> Any:
        data = json.dumps(request, allow_nan=False, separators=(",", ":"), ensure_ascii=False).encode()
        if len(data) > _LIMIT:
            raise ValueError("Workflow message exceeds 64 MiB")
        raw = self._instance.invoke(_CAP, operation, data, max_output_bytes=_LIMIT)
        try:
            reply = json.loads(raw)
            if isinstance(reply, dict) and len(reply) == 1:
                if "error" in reply:
                    raise WorkflowError(**reply["error"])
                return reply[field]
        except (ValueError, TypeError, KeyError):
            pass
        raise ProtocolError("Invalid workflow reply")

    def prepare(self, profile: bytes, pins: Pins, request: Mapping[str, Any], identity: Identity) -> Prepared:
        live = self._call("prepare", dict(profile_utf8=profile.decode(), pins=pins, request=dict(request), identity=identity), "prepared")
        return Prepared(self, live["state_ref"], live["record"])

    def prepare_file(self, profile: Path, pins: Pins, request: Mapping[str, Any], identity: Identity) -> Prepared:
        return self.prepare(profile.read_bytes(), pins, request, identity)

    def results(self, profile: bytes, pins: Pins, run_reference: str, state: str, outputs: Mapping[str, Any]) -> ResultRecord:
        return self._call("results", dict(profile_utf8=profile.decode(), pins=pins, run_reference=run_reference, state=state, outputs=dict(outputs)))

    def verify(self, record: ResultRecord, pointer: str, content: bytes, expected: Checksum | None = None) -> ResultRecord:
        return self._call("verify", dict(record=record, pointer=pointer, base64=base64.b64encode(content).decode(), expected=expected))

    def compare(self, policy: Policy, left: ResultRecord, right: ResultRecord) -> Report:
        return self._call("compare", dict(policy=policy, left=left, right=right))


class Prepared:
    def __init__(self, client: Workflow, key: str, record: PlanRecord):
        self._client, self._key, self.record = client, key, record
        self._fingerprint = record["fingerprint"]
        self._lock = Lock()

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def close(self) -> None:
        with self._lock:
            if self._key:
                self._client._call("release", {"state_ref": self._key}, "released")
                self._key = ""

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def submit(self, wes: WES, attachments: Mapping[str, bytes] | None = None) -> Receipt:
        files = [{"filename": name, "base64": base64.b64encode(data).decode()} for name, data in (attachments or {}).items()]
        with self._lock:
            key = self._key
            if not key:
                raise WorkflowError("invalid_state", "Preparation is closed", [])
            action = self._client._call("begin_submit", {"state_ref": key, "attachments": files})
            self._key = ""
        try:
            request = action["request"]
            if not isinstance(request, dict):
                raise ProtocolError("Expected an in-memory WES request")
            upload = [(item["filename"], base64.b64decode(item["base64"], validate=True)) for item in action["attachments"]]
            run = wes.run(request, upload)
        except BaseException:
            try:
                self._client._call("finish_submit", {"state_ref": key, "outcome": "failed"})
            except BaseException:
                self._release_quietly(key)
            raise
        run_id = run.id
        try:
            receipt = self._client._call("finish_submit", {"state_ref": key, "outcome": {"submitted": {"run_id": run_id}}})
            if receipt["run_id"] != run_id or receipt["invocation"]["fingerprint"] != self._fingerprint:
                raise ProtocolError("Invalid workflow receipt")
            return receipt
        except Exception:
            self._release_quietly(key)
            raise AcceptedRunError(run_id) from None

    def _release_quietly(self, key: str) -> None:
        try:
            self._client._call("release", {"state_ref": key}, "released")
        except BaseException:
            pass
