"""A recording TES- and WES-like HTTP mock in a separate process.

It prints its port on the first line of stdout. ``GET /__requests__`` returns
the requests received so far. The process keeps tests isolated from Python
runtime and GIL changes in the bindings. ``GET /api/tasks/slow-<seconds>``
answers with a running task of that ID after the given delay, for tests of
concurrent and cancelled calls, as does ``GET /slow-<seconds>/service-info``
with the service info (``GET /failing-<seconds>/service-info`` with a 503); if the
client closes the connection before,
``GET /__disconnects__`` lists the request's path. Below ``/hang``,
``service-info`` answers at once and every other request gets no answer until
the client leaves (at most 30 seconds), for tests of timed-out submissions.
A route can answer with a ``Raw`` body instead of JSON, or be a function of the
query parameters; ``{host}`` in a reply header becomes the request's host. Reply
headers are a ``dict`` or a list of pairs, which may repeat a name; a ``bytes``
value is sent as it is.
"""

import base64
import json
import select
import socket
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, quote, urlsplit

SERVICE_INFO = {
    "id": "org.example.tes",
    "name": "Example TES",
    "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
    "organization": {"name": "Example", "url": "https://example.org"},
    "version": "1.0.0",
}

TASK = {
    "id": "task-1",
    "state": "COMPLETE",
    "name": "hello",
    "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
    "tags": {"gateway.plugin": "example"},
    "extension_field": {"nested": [1, 2, 3]},
}

ROUTES = {
    ("GET", "/api/service-info"): (200, SERVICE_INFO),
    # A TES whose other endpoints all fail.
    ("GET", "/broken/service-info"): (200, SERVICE_INFO),
    ("GET", "/api/tasks/task-1"): (200, TASK),
    ("GET", "/api/tasks"): (200, {"tasks": [TASK], "next_page_token": "next"}),
    ("GET", "/api/tasks/task-1/provenance"): (200, {"events": ["created"]}),
    ("GET", "/api/tasks/bad"): (200, {"id": "bad", "executors": "not a list"}),
    ("GET", "/api/tasks/new-state"): (200, {"id": "new-state", "state": "SUSPENDED"}),
    ("POST", "/api/tasks"): (200, {"id": "task-1"}),
    ("POST", "/api/runs"): (200, {"run_id": "run-1"}),
    ("POST", "/api/tasks/task-1:cancel"): (200, {}),
    ("PUT", "/api/plugins/p1"): (200, {"updated": True}),
    ("DELETE", "/api/plugins/p1"): (200, {"deleted": True}),
}

# A DRS below `/drs`. Replies may carry headers as a third item.
DRS_SERVICE_INFO = {
    **SERVICE_INFO,
    "id": "org.example.drs",
    "name": "Example DRS",
    "type": {"group": "org.ga4gh", "artifact": "drs", "version": "1.4.0"},
}

DRS_OBJECT = {
    "id": "obj-1",
    "name": "reads.bam",
    "self_uri": "drs://drs.example/obj-1",
    "size": 1024,
    "created_time": "2026-01-01T00:00:00Z",
    "mime_type": "application/octet-stream",
    "checksums": [{"checksum": "abc", "type": "sha-256"}],
    "access_methods": [
        {"type": "s3", "access_url": {"url": "s3://bucket/obj-1"}, "region": "us-east-1"},
        {"type": "https", "access_id": "https-1", "region": "eu-west-1"},
    ],
    "aliases": ["reads"],
    "extension_field": {"nested": [1, 2, 3]},
}

ACCESS_URL = {
    "url": "https://store.example/obj-1?signature=SECRET",
    "headers": ["Authorization: Bearer SECRET"],
    "extension_field": "kept",
}

GEN3_REPLY = json.loads(
    (Path(__file__).resolve().parents[2] / "tests/fixtures/drs/gen3-caninedc-bulk-objects.json")
    .read_text()
)
GEN3_OBJECT = GEN3_REPLY["resolved_drs_objects"][0]


class Sequence:
    """Answers with each reply in turn, then keeps answering with the last one."""

    def __init__(self, *replies):
        self.replies = list(replies)

    def __iter__(self):
        reply = self.replies.pop(0) if len(self.replies) > 1 else self.replies[0]
        return iter(reply)


class Raw:
    """A reply body that is sent as it is, not as JSON."""

    def __init__(self, data, content_type="text/plain"):
        self.data = data
        self.content_type = content_type


PENDING = (202, {}, {"Retry-After": "1"})

ROUTES.update(
    {
        ("GET", "/drs/service-info"): (200, DRS_SERVICE_INFO),
        ("GET", "/drs/objects/obj-1"): (200, DRS_OBJECT),
        ("GET", "/drs/objects/obj-1/access/https-1"): (200, ACCESS_URL),
        ("GET", "/drs/objects/a%2Fb%20c"): (200, {**DRS_OBJECT, "id": "a/b c"}),
        ("GET", "/drs/objects/pending"): PENDING,
        ("GET", "/drs/objects/obj-1/access/pending"): PENDING,
        ("GET", "/drs/objects/later"): Sequence(PENDING, (200, {**DRS_OBJECT, "id": "later"})),
        ("GET", "/drs/objects/no-retry"): (202, {}),
        ("GET", "/drs/objects/bad"): (200, {"id": "bad"}),
        ("GET", "/drs/objects/obj-1/access/bad"): (200, ["not", "an", "object"]),
        ("GET", "/drs/objects/secret-access"): (
            200,
            {
                **DRS_OBJECT,
                "id": "secret-access",
                "access_methods": [{"type": "https", "access_id": "wrong-type"}],
            },
        ),
        ("GET", "/drs/objects/secret-access/access/wrong-type"): (
            200,
            {"url": "https://store.example/x", "headers": "Authorization: Bearer SECRET"},
        ),
        ("GET", "/drs/objects/obj-1/access/wrong-type"): (
            200,
            {"url": "https://store.example/x", "headers": "Authorization: Bearer SECRET"},
        ),
        ("POST", "/drs/objects"): (
            200,
            {
                "resolved_drs_object": [DRS_OBJECT, {"id": "invalid"}],
                "unresolved_drs_objects": [{"error_code": 404, "object_ids": ["missing"]}],
            },
        ),
        ("POST", "/drs/objects/access"): (
            200,
            {
                "resolved_drs_object_access_urls": [
                    {"drs_object_id": "obj-1", "drs_access_id": "https-1", **ACCESS_URL}
                ],
                "unresolved_drs_objects": [{"error_code": 403, "object_ids": ["obj-2"]}],
            },
        ),
        ("GET", "/drs-gen3/service-info"): (200, DRS_SERVICE_INFO),
        ("GET", f"/drs-gen3/objects/{quote(GEN3_OBJECT['id'], safe='')}"): (200, GEN3_OBJECT),
        ("POST", "/drs-gen3/objects"): (200, GEN3_REPLY),
        # A DRS whose bulk requests are still pending.
        ("GET", "/drs-pending/service-info"): (200, DRS_SERVICE_INFO),
        ("POST", "/drs-pending/objects"): (202, {}, {"Retry-After": "5"}),
    }
)

REQUESTS = []
DISCONNECTS = []


class Handler(BaseHTTPRequestHandler):
    """Records each request and answers it from `ROUTES`."""

    def _reply(self, status, payload, headers=None):
        content_type = "application/json"
        if isinstance(payload, Raw):
            data, content_type = payload.data, payload.content_type
        else:
            data = json.dumps(payload).encode()
        self.send_response(status)
        pairs = headers.items() if isinstance(headers, dict) else headers or []
        for name, value in pairs:
            if isinstance(value, bytes):  # A header value that isn't text.
                self._headers_buffer.append(name.encode() + b": " + value + b"\r\n")
                continue
            self.send_header(name, value.replace("{host}", self.headers.get("Host", "")))
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _handle(self):
        url = urlsplit(self.path)
        if url.path == "/__requests__":
            self._reply(200, REQUESTS)
            return
        if url.path == "/__disconnects__":
            self._reply(200, DISCONNECTS)
            return
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        REQUESTS.append(
            {
                "method": self.command,
                "path": url.path,
                "query": parse_qs(url.query, keep_blank_values=True),
                "raw_query": url.query,
                "headers": dict(self.headers),
                "body": base64.b64encode(body).decode(),
            }
        )
        if self.command == "GET" and url.path.startswith("/api/tasks/slow-"):
            task_id = url.path.rsplit("/", 1)[1]
            if self._client_left_within(float(task_id.removeprefix("slow-"))):
                DISCONNECTS.append(url.path)
                return
            self._reply(200, {**TASK, "id": task_id, "state": "RUNNING"})
            return
        if url.path.startswith("/failing-") and url.path.endswith("/service-info"):
            time.sleep(float(url.path.split("/")[1].removeprefix("failing-")))
            self._reply(503, {"msg": "unavailable"})
            return
        if url.path == "/hang/service-info":
            self._reply(200, SERVICE_INFO)
            return
        if url.path.startswith("/hang/"):
            self._client_left_within(30)
            return
        if url.path.startswith("/slow-") and url.path.endswith("/service-info"):
            if self._client_left_within(float(url.path.split("/")[1].removeprefix("slow-"))):
                DISCONNECTS.append(url.path)
                return
            self._reply(200, SERVICE_INFO)
            return
        reply = ROUTES.get((self.command, url.path), (404, {"msg": "not found"}))
        if callable(reply):
            reply = reply(parse_qs(url.query, keep_blank_values=True))
        self._reply(*reply)

    def _client_left_within(self, seconds):
        """Waits up to `seconds`; tells whether the client closed the connection meanwhile."""
        deadline = time.monotonic() + seconds
        while (remaining := deadline - time.monotonic()) > 0:
            readable, _, _ = select.select([self.connection], [], [], min(remaining, 0.05))
            if readable:
                try:
                    if not self.connection.recv(1, socket.MSG_PEEK):
                        return True
                except ConnectionResetError:
                    return True
                time.sleep(min(remaining, 0.05))  # Pipelined data, not a close.
        return False

    do_GET = do_POST = do_PUT = do_DELETE = _handle

    def log_message(self, *args):
        pass


# A WES below `/wes`.
WES_SERVICE_INFO = {
    **SERVICE_INFO,
    "id": "org.example.wes",
    "name": "Example WES",
    "type": {"group": "org.ga4gh", "artifact": "wes", "version": "1.1.0"},
}

RUN_REQUEST = {
    "workflow_type": "CWL",
    "workflow_type_version": "v1.2",
    "workflow_url": "main.cwl",
    "workflow_params": {"message": "hello"},
    "tags": {"project": "demo"},
}

RUN_LOG = {
    "run_id": "run-1",
    "state": "COMPLETE",
    "request": RUN_REQUEST,
    "run_log": {"name": "hello", "exit_code": 0},
    "task_logs_url": "runs/run-1/tasks",
    "outputs": {"out": "hello"},
    "extension_field": {"nested": [1, 2, 3]},
}

RUN_SUMMARY = {
    "run_id": "run-1",
    "state": "COMPLETE",
    "start_time": "2026-01-01T00:00:00Z",
    "tags": {"project": "demo"},
    "extension_field": True,
}

TASK_LOG = {
    "id": "task-1",
    "name": "echo",
    "cmd": ["echo", "hello"],
    "exit_code": 0,
    "tes_uri": "https://tes.example/v1/tasks/1",
    "extension_field": "kept",
}

ROUTES.update(
    {
        ("GET", "/wes/service-info"): (200, WES_SERVICE_INFO),
        ("POST", "/wes/runs"): (200, {"run_id": "run-1"}),
        ("GET", "/wes/runs"): (200, {"runs": [RUN_SUMMARY], "next_page_token": "next"}),
        ("GET", "/wes/runs/run-1"): (200, RUN_LOG),
        ("GET", "/wes/runs/run-1/status"): (200, {"run_id": "run-1", "state": "COMPLETE"}),
        ("POST", "/wes/runs/run-1/cancel"): (200, {"run_id": "run-1"}),
        ("GET", "/wes/runs/run-1/tasks"): (200, {"task_logs": [TASK_LOG], "next_page_token": "next"}),
        ("GET", "/wes/runs/run-1/tasks/task-1"): (200, TASK_LOG),
        ("GET", "/wes/runs/no-state"): (200, {"run_id": "no-state"}),
        ("GET", "/wes/runs/bad"): (200, {"run_id": "bad", "state": "NOT_A_STATE"}),
        ("GET", "/wes/runs/malformed"): (200, ["not", "an", "object"]),
        # A WES 0.2.0 server (GA4GH Starter Kit) lists runs as a bare array.
        ("GET", "/starter-kit/service-info"): (200, WES_SERVICE_INFO),
        ("GET", "/starter-kit/runs"): (200, [RUN_SUMMARY]),
        # Responses that don't fit the WES models.
        ("GET", "/wes/runs/bad/status"): (200, {"run_id": "bad", "state": "NOT_A_STATE"}),
        ("GET", "/wes/runs/bad/tasks"): (200, {"task_logs": [{"id": 1}]}),
        ("GET", "/wes/runs/bad/tasks/t"): (200, ["not", "an", "object"]),
        ("GET", "/wes-bad-list/service-info"): (200, WES_SERVICE_INFO),
        ("GET", "/wes-bad-list/runs"): (200, {"runs": [{"state": "COMPLETE"}]}),
    }
)

# TRS 2.0.1 below `/trs`, TRS 2.0.2 below `/trs2`, and a TRS that declares a version that
# the SDK doesn't know below `/trs-next`. IDs that need encoding check that they reach the
# server unchanged, as one path segment each.
TRS_TOOL_ID = "#workflow/github.com/org/repo"
TRS_VERSION_ID = "v1/rc 1"
TRS_VERSION_PATH = f"tools/{quote(TRS_TOOL_ID, safe='')}/versions/{quote(TRS_VERSION_ID, safe='')}"

TRS_VERSION = {
    "url": "https://trs.example/tools/1/versions/1",
    "id": TRS_VERSION_ID,
    "name": "First release",
    "author": ["Ada"],
    "descriptor_type": ["CWL"],
    "descriptor_type_version": {"CWL": ["v1.2"]},
    "images": [{"registry_host": "quay.io", "image_name": "org/tool", "image_type": "Docker"}],
    "is_production": True,
    "verified": False,
    "description": "The first release",
    "extension_field": {"nested": [1, 2, 3]},
}

TRS_TOOL = {
    "url": "https://trs.example/tools/1",
    "id": TRS_TOOL_ID,
    "aliases": ["repo"],
    "organization": "org",
    "name": "repo",
    "toolclass": {"id": "1", "name": "Workflow", "extension_field": "kept"},
    "description": "A workflow",
    "has_checker": False,
    "versions": [TRS_VERSION],
    "extension_field": {"nested": [1, 2, 3]},
}

TRS_DESCRIPTOR = {
    "content": "cwlVersion: v1.2\nclass: Workflow\n",
    "url": "https://trs.example/raw/main.cwl",
    "checksum": [{"checksum": "abc", "type": "sha-256"}],
    "image_type": "CWL",
    "extension_field": "kept",
}

TRS_PLAIN_DESCRIPTOR = "cwlVersion: v1.2\nclass: Workflow\n"

TRS_FILES = [
    {"path": "main.cwl", "file_type": "PRIMARY_DESCRIPTOR", "extension_field": "kept"},
    {"path": "tools/align.cwl", "file_type": "SECONDARY_DESCRIPTOR"},
    {"path": "notes.md", "file_type": "NOT_A_TRS_TYPE"},
]

# The smallest valid ZIP archive: an end-of-central-directory record.
TRS_ZIP = b"PK\x05\x06" + bytes(18)


def trs_service_info(version):
    return {
        **SERVICE_INFO,
        "id": "org.example.trs",
        "name": "Example TRS",
        "type": {"group": "org.ga4gh", "artifact": "trs", "version": version},
    }


def trs_tools(root):
    """Two pages of one tool each below `root`; the first links to the second."""

    def reply(query):
        if query.get("offset") == ["1"]:
            return (200, [{**TRS_TOOL, "id": "second"}], {"current_offset": "1"})
        return (
            200,
            [TRS_TOOL],
            {
                "next_page": f"http://{{host}}/{root}/tools?offset=1&limit=1",
                "last_page": f"http://{{host}}/{root}/tools?offset=1&limit=1",
                "current_offset": "0",
                "current_limit": "1",
            },
        )

    return reply


def trs_files(query):
    """The file list, or the ZIP archive for `format=zip`."""
    if query.get("format") == ["zip"]:
        return (200, Raw(TRS_ZIP, "application/zip"))
    return (200, TRS_FILES)


for root, version in [("trs", "2.0.1"), ("trs2", "2.0.2"), ("trs-next", "9.9.9")]:
    tool_path = f"/{root}/tools/{quote(TRS_TOOL_ID, safe='')}"
    version_path = f"/{root}/{TRS_VERSION_PATH}"
    ROUTES.update(
        {
            ("GET", f"/{root}/service-info"): (200, trs_service_info(version)),
            ("GET", f"/{root}/toolClasses"): (200, [TRS_TOOL["toolclass"]]),
            ("GET", f"/{root}/tools"): trs_tools(root),
            ("GET", tool_path): (200, TRS_TOOL),
            ("GET", f"{tool_path}/versions"): (200, [TRS_VERSION]),
            ("GET", version_path): (200, TRS_VERSION),
            ("GET", f"{version_path}/CWL/descriptor"): (200, TRS_DESCRIPTOR),
            ("GET", f"{version_path}/PLAIN_CWL/descriptor"): (
                200,
                Raw(TRS_PLAIN_DESCRIPTOR.encode()),
            ),
            ("GET", f"{version_path}/CWL/descriptor/tools%2Falign.cwl"): (
                200,
                {**TRS_DESCRIPTOR, "content": "class: CommandLineTool\n"},
            ),
            ("GET", f"{version_path}/CWL/tests"): (200, [TRS_DESCRIPTOR]),
            ("GET", f"{version_path}/PLAIN_CWL/tests"): (200, Raw(b"message: hello\n")),
            ("GET", f"{version_path}/CWL/files"): trs_files,
            ("GET", f"{version_path}/containerfile"): (
                200,
                [{"content": "FROM alpine\n", "image_type": "Docker"}],
            ),
            # Responses that don't fit the TRS models, and a registry that ignores
            # `format=zip`.
            ("GET", f"/{root}/tools/bad"): (200, {"id": "bad"}),
            ("GET", f"/{root}/tools/bad-description"): (
                200,
                {**TRS_TOOL, "id": "bad-description", "versions": [{**TRS_VERSION, "description": 7}]},
            ),
            ("GET", f"/{root}/tools/t/versions/1/CWL/files"): (200, TRS_FILES),
            # Repeated, empty and non-text paging headers.
            ("GET", f"/{root}/odd-headers/tools"): (
                200,
                [TRS_TOOL],
                [
                    ("next_page", "http://{host}/trs/tools?offset=1"),
                    ("next_page", "http://{host}/trs/tools?offset=2"),
                    ("last_page", ""),
                    ("self_link", b"http://h/\xff"),
                ],
            ),
        }
    )

# WES roots whose `POST runs` answers with a failure or a malformed acknowledgement.
BAD_ACKNOWLEDGEMENTS = {
    "ack-status": (500, {"msg": "failed"}),
    "ack-empty": (200, {}),
    "ack-null": (200, {"run_id": None}),
    "ack-number": (200, {"run_id": 4}),
    "ack-blank": (200, {"run_id": ""}),
    "ack-text": (200, "not an object"),
}
for root, reply in BAD_ACKNOWLEDGEMENTS.items():
    ROUTES[("GET", f"/{root}/service-info")] = (200, WES_SERVICE_INFO)
    ROUTES[("POST", f"/{root}/runs")] = reply

if __name__ == "__main__":
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    print(httpd.server_address[1], flush=True)
    httpd.serve_forever()
