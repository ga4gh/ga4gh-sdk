"""The DRS bindings: objects, access URLs, pending answers, bulk calls and DRS URIs."""

import base64
import json
import threading
import time
import urllib.request

import pytest

import GA4GH
from GA4GH import (
    AccessPlan,
    AccessURL,
    BulkAccessURL,
    BulkOutcome,
    BulkPolicy,
    BulkReport,
    Configuration,
    DRS,
    DrsObject,
    DrsResolver,
    DrsUri,
    PendingResolution,
    Transport,
)

from .mock_server import ACCESS_URL, DRS_OBJECT


@pytest.fixture
def drs_config(server):
    return Configuration(f"{server.root}/drs")


@pytest.fixture
def drs(drs_config):
    return DRS(drs_config)


def body(request):
    """The JSON body of a recorded request."""
    return json.loads(request["body"])


def test_client_reads_service_info_and_exposes_its_configuration(server, drs, drs_config):
    assert server.last()["path"] == "/drs/service-info"
    assert drs.config.get_base_path() == drs_config.get_base_path()
    assert isinstance(drs.transport, Transport)
    assert json.loads(drs.transport.get("objects/obj-1"))["id"] == "obj-1"


def test_client_rejects_other_services_and_wrong_roots(server, config):
    with pytest.raises(RuntimeError, match="not an instance of DRS"):
        DRS(config)
    with pytest.raises(RuntimeError, match="404.*Check that base_path is the DRS API root"):
        DRS(Configuration(f"{server.root}/missing"))


def test_get_object_keeps_full_json(server, drs):
    obj = drs.get_object("obj-1")
    assert server.last()["path"] == "/drs/objects/obj-1"
    assert isinstance(obj, DrsObject)
    assert obj.to_dict() == DRS_OBJECT
    assert json.loads(obj.json()) == DRS_OBJECT
    assert obj.to_dict() is not obj.to_dict()
    assert (obj.id, obj.name, obj.self_uri, obj.size) == ("obj-1", "reads.bam", DRS_OBJECT["self_uri"], 1024)
    assert obj.created_time == "2026-01-01T00:00:00Z"
    assert obj.mime_type == "application/octet-stream"
    assert obj.checksums == DRS_OBJECT["checksums"]
    assert obj.access_methods == DRS_OBJECT["access_methods"]
    assert obj.aliases == ["reads"]
    assert (obj.updated_time, obj.version, obj.contents, obj.description) == (None, None, None, None)
    assert repr(obj) == "DrsObject(id='obj-1', size=1024)"


def test_ids_are_encoded_as_one_path_segment(server, drs):
    assert drs.get_object("a/b c").id == "a/b c"
    assert server.last()["path"] == "/drs/objects/a%2Fb%20c"


@pytest.mark.parametrize("object_id", ["", ".", ".."])
def test_invalid_ids_raise_before_a_request(server, drs, object_id):
    before = server.last()
    with pytest.raises(ValueError, match="object ID"):
        drs.get_object(object_id)
    with pytest.raises(ValueError, match="object ID"):
        drs.get_access_url(object_id, "https-1")
    with pytest.raises(ValueError, match="access ID"):
        drs.get_access_url_once("obj-1", object_id)
    assert server.last() == before


def test_get_access_url_keeps_full_json_and_hides_credentials(server, drs):
    access = drs.get_access_url("obj-1", "https-1")
    assert server.last()["path"] == "/drs/objects/obj-1/access/https-1"
    assert isinstance(access, AccessURL)
    assert access.url == ACCESS_URL["url"]
    assert access.headers == ACCESS_URL["headers"]
    assert access.to_dict() == ACCESS_URL
    assert json.loads(access.json()) == ACCESS_URL
    assert "SECRET" not in repr(access)
    assert repr(access) == "AccessURL(url=<redacted>, headers=1)"


def test_invalid_responses_raise_runtime_error(drs):
    with pytest.raises(RuntimeError, match="Invalid DRS response"):
        drs.get_object("bad")
    with pytest.raises(RuntimeError, match="Invalid DRS response: not a JSON object"):
        drs.get_access_url("obj-1", "bad")
    with pytest.raises(RuntimeError, match="404"):
        drs.get_object("unknown")
    with pytest.raises(RuntimeError, match="Invalid DRS response: missing field `self_uri`"):
        drs.get_object("bad")


def test_model_errors_quote_no_response_values(drs):
    with pytest.raises(RuntimeError) as error:
        drs.get_access_url("obj-1", "wrong-type")
    assert "SECRET" not in str(error.value)
    assert "wrong type" in str(error.value)


def test_resolver_errors_quote_no_response_values(drs_config):
    resolver = DrsResolver().with_service("drs.example", Transport(drs_config))
    for call in (resolver.access_plan_once, resolver.access_plan):
        with pytest.raises(RuntimeError, match="Invalid DRS response: a field has the wrong") as error:
            call("drs://drs.example/secret-access")
        assert "SECRET" not in str(error.value)


def test_pending_answers(server, drs):
    with pytest.raises(RuntimeError, match="pending; retry after 1 seconds"):
        drs.get_object("pending")
    with pytest.raises(RuntimeError, match="pending; retry after 1 seconds"):
        drs.get_access_url("obj-1", "pending")

    pending = drs.get_object_once("pending")
    assert isinstance(pending, PendingResolution)
    assert pending.retry_after == 1
    assert pending.endpoint == "objects/pending"
    assert pending.api_root == f"{server.root}/drs/"
    assert pending.to_dict() == {
        "api_root": f"{server.root}/drs/",
        "endpoint": "objects/pending",
        "retry_after": 1,
    }
    assert repr(pending) == "PendingResolution(endpoint='objects/pending', retry_after=1)"
    assert drs.get_access_url_once("obj-1", "pending").endpoint == "objects/obj-1/access/pending"

    assert isinstance(drs.get_object_once("obj-1"), DrsObject)
    assert isinstance(drs.get_access_url_once("obj-1", "https-1"), AccessURL)

    with pytest.raises(RuntimeError, match="Retry-After"):
        drs.get_object_once("no-retry")


def test_resolve_waits_while_pending(server, drs):
    start = time.monotonic()
    obj = drs.resolve_object("later", timeout=10)
    assert time.monotonic() - start >= 1
    assert obj.id == "later"
    assert obj.to_dict()["extension_field"] == DRS_OBJECT["extension_field"]
    assert drs.resolve_access_url("obj-1", "https-1").to_dict() == ACCESS_URL


def test_resolve_gives_up(drs):
    with pytest.raises(RuntimeError, match="attempts exhausted.*retry after 1 seconds"):
        drs.resolve_object("pending", max_attempts=1)
    with pytest.raises(RuntimeError, match="deadline exceeded"):
        drs.resolve_access_url("obj-1", "pending", timeout=0.5, max_attempts=3)


@pytest.mark.parametrize(
    "kwargs, message",
    [
        ({"timeout": 0}, "timeout"),
        ({"timeout": -1}, "timeout"),
        ({"timeout": float("nan")}, "timeout"),
        ({"timeout": 1e300}, "timeout"),
        ({"max_attempts": 0}, "max_attempts"),
        ({"max_attempts": 10_001}, "max_attempts"),
        ({"max_attempts": -1}, "max_attempts"),
    ],
)
def test_resolve_checks_its_limits_before_a_request(server, drs, kwargs, message):
    before = server.last()
    with pytest.raises(ValueError, match=message):
        drs.resolve_object("obj-1", **kwargs)
    assert server.last() == before


def test_bulk_objects(server, drs):
    report = drs.get_bulk_objects(["obj-1", "missing", "invalid", "obj-1"], passports=["passport"])
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/drs/objects")
    assert body(request) == {
        "bulk_object_ids": ["invalid", "missing", "obj-1"],
        "passports": ["passport"],
    }
    assert isinstance(report, BulkReport)
    assert report.diagnostics == []
    assert repr(report) == "BulkReport(outcomes=3, diagnostics=0)"
    invalid, missing, found = report.outcomes
    assert isinstance(found, BulkOutcome)
    assert (found.object_id, found.access_id, found.is_success) == ("obj-1", None, True)
    [obj] = found.resolved
    assert isinstance(obj, DrsObject)
    assert obj.to_dict() == DRS_OBJECT
    assert (found.failure_codes, found.pending, found.errors) == ([], None, [])
    assert (missing.is_success, missing.resolved, missing.failure_codes) == (False, [], [404])
    assert invalid.is_success is False
    assert invalid.resolved == []
    assert [error.split(":")[0] for error in invalid.errors] == ["invalid resolved entry"]
    assert repr(missing) == (
        "BulkOutcome(object_id='missing', resolved=0, failure_codes=[404], pending=False, errors=0)"
    )


def test_bulk_access_urls(server, drs):
    report = drs.get_bulk_access_urls({"obj-1": "https-1", "obj-2": "https-2"})
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/drs/objects/access")
    assert body(request) == {
        "bulk_object_access_ids": [
            {"bulk_object_id": "obj-1", "bulk_access_ids": ["https-1"]},
            {"bulk_object_id": "obj-2", "bulk_access_ids": ["https-2"]},
        ]
    }
    found, denied = report.outcomes
    assert (found.object_id, found.access_id, found.is_success) == ("obj-1", "https-1", True)
    [access] = found.resolved
    assert isinstance(access, BulkAccessURL)
    assert (access.drs_object_id, access.drs_access_id) == ("obj-1", "https-1")
    assert (access.url, access.headers) == (ACCESS_URL["url"], ACCESS_URL["headers"])
    assert access.to_dict()["extension_field"] == "kept"
    assert json.loads(access.json()) == access.to_dict()
    assert "SECRET" not in repr(access) + repr(found) + repr(report)
    assert (denied.access_id, denied.failure_codes) == ("https-2", [403])
    assert drs.get_bulk_access_urls([("obj-1", "https-1")]).outcomes[0].is_success


def test_bulk_pending_and_empty_calls(server):
    drs = DRS(Configuration(f"{server.root}/drs-pending"))
    [outcome] = drs.get_bulk_objects(["obj-1"]).outcomes
    assert outcome.is_success is False
    assert (outcome.pending.endpoint, outcome.pending.retry_after) == ("objects", 5)
    assert repr(outcome).endswith("pending=True, errors=0)")
    before = server.last()
    assert drs.get_bulk_objects([]).outcomes == []
    assert server.last() == before


@pytest.mark.parametrize(
    "call, message",
    [
        (lambda drs: drs.get_bulk_objects("obj-1"), "ids must be an iterable of str"),
        (lambda drs: drs.get_bulk_objects([1]), "ids must be an iterable of str"),
        (lambda drs: drs.get_bulk_objects([""]), "nonempty"),
        (lambda drs: drs.get_bulk_objects(["a"], passports="p"), "passports"),
        (lambda drs: drs.get_bulk_objects(["a"], timeout=0), "timeout"),
        (lambda drs: drs.get_bulk_access_urls("obj-1"), "pairs"),
        (lambda drs: drs.get_bulk_access_urls([("obj-1",)]), "pairs"),
        (
            lambda drs: drs.get_bulk_access_urls([("obj-1", "a"), ("obj-1", "b")]),
            "one access ID per object",
        ),
        (
            lambda drs: drs.get_bulk_objects(["a"], policy=BulkPolicy(chunk_size=0)),
            "bulk limits",
        ),
        (
            lambda drs: drs.get_bulk_objects(["a"], policy=BulkPolicy(concurrency=65)),
            "bulk limits",
        ),
        (
            lambda drs: drs.get_bulk_objects(["a"], policy=BulkPolicy(max_attempts=0)),
            "Invalid bulk request",
        ),
    ],
)
def test_invalid_bulk_input_raises_before_a_request(server, drs, call, message):
    before = server.last()
    with pytest.raises(ValueError, match=message):
        call(drs)
    assert server.last() == before


def test_bulk_policy():
    policy = BulkPolicy()
    assert (policy.chunk_size, policy.advertised_limit, policy.concurrency) == (100, None, 4)
    assert (policy.max_requests, policy.max_split_depth, policy.max_attempts) == (1000, 8, 1)
    assert (policy.initial_delay, policy.max_delay) == (0.2, 10.0)
    policy = BulkPolicy(chunk_size=10, concurrency=2, max_attempts=3, initial_delay=0.5)
    assert (policy.chunk_size, policy.concurrency, policy.max_attempts) == (10, 2, 3)
    assert policy.initial_delay == 0.5
    limited = policy.with_service_info({"id": "x", "maxBulkRequestLength": 5})
    assert (limited.advertised_limit, limited.chunk_size, policy.advertised_limit) == (5, 10, None)
    assert policy.with_service_info('{"id": "x"}').advertised_limit is None
    with pytest.raises(ValueError, match="maxBulkRequestLength"):
        policy.with_service_info({"maxBulkRequestLength": 0})
    with pytest.raises(ValueError, match="initial_delay"):
        BulkPolicy(initial_delay=-1)
    for name in ["chunk_size", "advertised_limit", "concurrency", "max_requests",
                 "max_split_depth", "max_attempts"]:
        with pytest.raises(ValueError, match=f"{name} must not be negative"):
            BulkPolicy(**{name: -1})


def recorded_bodies(server, path):
    """The JSON bodies of all requests to `path` that the mock server has received."""
    with urllib.request.urlopen(f"{server.root}/__requests__", timeout=10) as response:
        recorded = json.load(response)
    return [json.loads(base64.b64decode(r["body"])) for r in recorded if r["path"] == path]


def test_bulk_policy_limits_requests(server, drs):
    drs.get_bulk_objects(["a", "b", "c"], policy=BulkPolicy(chunk_size=2, concurrency=1))
    assert recorded_bodies(server, "/drs/objects")[-2:] == [
        {"bulk_object_ids": ["a", "b"]},
        {"bulk_object_ids": ["c"]},
    ]


def test_drs_uri():
    uri = DrsUri("drs://DRS.Example/a%2Fb")
    assert (uri.authority, uri.object_id) == ("drs.example", "a/b")
    assert uri.default_api_root() == "https://drs.example/ga4gh/drs/v1"
    assert repr(uri) == "DrsUri(authority='drs.example', object_id='a/b')"
    for invalid in ["https://drs.example/a", "drs://prefix:accession", "drs://drs.example/", "drs://host:8080/a"]:
        with pytest.raises(ValueError, match="Invalid DRS URI"):
            DrsUri(invalid)


@pytest.fixture
def resolver(drs_config):
    resolver = DrsResolver()
    assert resolver.with_service("DRS.example", Transport(drs_config)) is resolver
    return resolver


def test_access_plan_with_access_id(server, resolver):
    plan = resolver.access_plan("drs://drs.example/obj-1", protocols=["HTTPS"])
    assert server.last()["path"] == "/drs/objects/obj-1/access/https-1"
    assert isinstance(plan, AccessPlan)
    assert (plan.object_id, plan.method, plan.region) == ("obj-1", "https", "eu-west-1")
    assert plan.url == ACCESS_URL["url"]
    assert plan.headers == [("authorization", "Bearer SECRET")]
    assert "SECRET" not in repr(plan)
    assert repr(plan) == "AccessPlan(object_id='obj-1', method='https', origin='https://store.example')"


def test_access_plan_with_direct_url(server, resolver):
    plan = resolver.access_plan_once("drs://drs.example/obj-1", regions=["us-east-1"])
    assert server.last()["path"] == "/drs/objects/obj-1"
    assert (plan.method, plan.url, plan.headers) == ("s3", "s3://bucket/obj-1", [])
    assert resolver.access_plan("drs://drs.example/obj-1", method_index=1).method == "https"


def test_access_plan_pending_and_failures(server, resolver):
    pending = resolver.access_plan_once("drs://drs.example/pending")
    assert isinstance(pending, PendingResolution)
    assert pending.endpoint == "objects/pending"
    with pytest.raises(RuntimeError, match="attempts exhausted"):
        resolver.access_plan("drs://drs.example/pending", max_attempts=1)
    with pytest.raises(RuntimeError, match="UnsupportedMethods"):
        resolver.access_plan("drs://drs.example/obj-1", protocols=["gs"])
    with pytest.raises(RuntimeError, match="InvalidOverride"):
        resolver.access_plan_once("drs://drs.example/obj-1", method_index=5)
    before = server.last()
    with pytest.raises(ValueError, match="access method type"):
        resolver.access_plan("drs://drs.example/obj-1", protocols=["carrier-pigeon"])
    with pytest.raises(ValueError, match="Invalid DRS URI"):
        resolver.access_plan_once("drs://prefix:accession")
    with pytest.raises(ValueError, match="max_attempts"):
        resolver.access_plan("drs://drs.example/obj-1", max_attempts=0)
    with pytest.raises(ValueError, match="method_index must not be negative"):
        resolver.access_plan_once("drs://drs.example/obj-1", method_index=-1)
    assert server.last() == before
    with pytest.raises(ValueError, match="Invalid DRS host"):
        resolver.with_service("host:8080", Transport(Configuration(server.base)))
    # A failed `with_service` keeps the services added before.
    assert resolver.access_plan_once("drs://drs.example/obj-1").method == "s3"


def recorded_requests(server, path):
    """The requests to `path` that the mock server has received."""
    with urllib.request.urlopen(f"{server.root}/__requests__", timeout=10) as response:
        return [r for r in json.load(response) if r["path"] == path]


def test_with_service_while_the_resolver_is_in_use(server, drs_config, resolver):
    errors = []

    def plan():
        # Waits about a second for the pending object, without the GIL.
        try:
            resolver.access_plan("drs://drs.example/pending", max_attempts=2)
        except RuntimeError as error:
            errors.append(error)

    asked = len(recorded_requests(server, "/drs/objects/pending"))
    thread = threading.Thread(target=plan)
    thread.start()
    # Once the server has the first request, the planning call holds the resolver until
    # it gives up, after the 1-second Retry-After.
    deadline = time.monotonic() + 10
    while len(recorded_requests(server, "/drs/objects/pending")) == asked:
        assert time.monotonic() < deadline
        time.sleep(0.01)
    with pytest.raises(RuntimeError, match="in use by another thread"):
        resolver.with_service("other.example", Transport(drs_config))
    thread.join(timeout=10)
    assert [str(e).split(":")[0] for e in errors] == ["Failed to plan DRS access"]
    assert resolver.with_service("other.example", Transport(drs_config)) is resolver


def test_unmapped_hosts_are_asked_anonymously_over_https():
    # `.invalid` names never resolve (RFC 6761), so the request fails.
    with pytest.raises(RuntimeError, match="Failed to plan DRS access"):
        DrsResolver().access_plan_once("drs://host.invalid/obj-1", timeout=10)


def test_client_can_be_used_from_several_threads(drs):
    results = []

    def get():
        results.append(drs.get_object("obj-1").id)

    threads = [threading.Thread(target=get) for _ in range(4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=10)
    assert results == ["obj-1"] * 4


@pytest.mark.parametrize(
    "cls",
    [
        GA4GH.DRS,
        GA4GH.DrsObject,
        GA4GH.AccessURL,
        GA4GH.BulkAccessURL,
        GA4GH.PendingResolution,
        GA4GH.BulkPolicy,
        GA4GH.BulkOutcome,
        GA4GH.BulkReport,
        GA4GH.DrsUri,
        GA4GH.DrsResolver,
        GA4GH.AccessPlan,
    ],
)
def test_classes_can_be_subclassed(cls):
    subclass = type(f"My{cls.__name__}", (cls,), {})
    assert issubclass(subclass, cls)


def test_subclasses_can_be_built(drs_config):
    policy = type("MyPolicy", (BulkPolicy,), {})(chunk_size=5)
    assert (type(policy).__name__, policy.chunk_size) == ("MyPolicy", 5)
    uri = type("MyUri", (DrsUri,), {})("drs://drs.example/a")
    assert (type(uri).__name__, uri.object_id) == ("MyUri", "a")
    drs = type("MyDRS", (DRS,), {})(drs_config)
    assert drs.get_object("obj-1").id == "obj-1"


class ObjectCountingDRS(DRS):
    """A downstream client that adds an endpoint and takes an extra argument."""

    def __init__(self, config, label="default"):
        super().__init__()
        self.label = label

    def object_names(self, *ids):
        return [json.loads(self.transport.get(f"objects/{i}"))["name"] for i in ids]


def test_subclass_with_extra_arguments_and_endpoint(drs_config):
    drs = ObjectCountingDRS(drs_config, label="mine")
    assert drs.label == "mine"
    assert drs.object_names("obj-1") == ["reads.bam"]
    assert drs.get_object("obj-1").id == "obj-1"
    with pytest.raises(TypeError, match="unexpected arguments"):
        DRS(drs_config, "extra")
    resolver = type("MyResolver", (DrsResolver,), {})()
    assert type(resolver.with_service("drs.example", Transport(drs_config))).__name__ == "MyResolver"
