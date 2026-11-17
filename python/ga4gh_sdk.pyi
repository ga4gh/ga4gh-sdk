"""Python clients for GA4GH TES, WES, DRS, TRS and Service Info APIs.

Build a `Configuration` with the service's API root, then a client for its API: `TES`
(or `AsyncTES` for `asyncio`), `WES`, `DRS` (and `DrsResolver` for `drs://` URIs) or
`TRS`. `ServiceInfo` reads any service's service info, and `Transport` sends requests to
further endpoints. The clients return typed result objects that keep the server's full
JSON, including fields that the SDK's models don't know.

All classes except the enums can be subclassed. Synchronous network methods release the
GIL while they wait. Failed requests and responses that don't fit the API's models raise
`RuntimeError`; invalid arguments raise `ValueError` before a request is sent.

Example:
    ```python
    from ga4gh_sdk import Configuration, TES

    tes = TES(Configuration("http://localhost:8000/ga4gh/tes/v1"))
    task = tes.create({"executors": [{"image": "alpine", "command": ["echo", "hi"]}]})
    print(task.wait(timeout=600))
    ```
"""

# Type stubs of the module. `tests/test_stubs.py` checks that they declare every class
# and member of it and document them; `api.md` renders them as the Python API reference
# (see `mkdocs.yml`).

import asyncio
import enum
import os
from collections.abc import Callable, Iterable, Mapping, Sequence
from typing import Any, Generic, Self, TypeAlias, TypeVar, final, overload

_JSONObject: TypeAlias = dict[str, Any]
"""A JSON object as a `dict`, for example a task's executor."""

_Params: TypeAlias = Mapping[str, Any] | str
"""Query parameters: a mapping or a JSON object string."""

@final
class ServiceType:
    """A built-in section of the configuration file, for `Configuration.from_file()`.

    Other sections are named by a `str`.
    """

    TES: ServiceType
    """The `TES` section, for `TES` and `AsyncTES`."""
    DRS: ServiceType
    """The `DRS` section, for `DRS`."""
    TRS: ServiceType
    """The `TRS` section, for `TRS`."""
    AAI: ServiceType
    """The `AAI` section (authentication and authorization infrastructure)."""
    WES: ServiceType
    """The `WES` section, for `WES`."""
    REGISTRY: ServiceType
    """The `REGISTRY` section (Service Registry)."""

class Configuration:
    """The API root and settings for connecting to a GA4GH service.

    `from_file()` reads a section of `~/.ga4gh-cli/config.json`, named by a
    `ServiceType` or by a `str`; in a file with profiles it reads the default
    profile. `get_extra()` returns the section's other keys.
    `set_timeouts()` and `get_timeouts()` set and give the time limits of
    requests in seconds; clients and transports read them when they are built.
    `set_token_provider()` sets a callable that gives a bearer token for each
    request.

    Set one source of credentials before building a client: Basic credentials, an
    OAuth or a generic bearer token, or a token provider. Clients and `Transport` take
    the settings when they are built, so later changes don't affect them.
    """

    def __init__(self, base_path: str) -> None:
        """Builds a configuration for the service at `base_path`.

        Args:
            base_path: The API root, including the service's path prefix, such as
                `"https://example.org/ga4gh/tes/v1"`.

        Raises:
            ValueError: `base_path` is not a valid URL.
        """
    def set_base_path(self, base_path: str) -> None:
        """Replaces the API root.

        Args:
            base_path: The new API root, including the service's path prefix.

        Raises:
            ValueError: `base_path` is not a valid URL.
        """
    def get_base_path(self) -> str:
        """Gives the API root.

        Returns:
            The API root as a URL string.
        """
    def set_basic_auth(self, username: str, password: str | None = None) -> None:
        """Sets one Basic credential for clients built from this configuration.

        Args:
            username: The user name.
            password: The password, if any.
        """
    def set_oauth_access_token(self, token: str) -> None:
        """Sets one OAuth bearer token for clients built from this configuration.

        Args:
            token: The access token, without the `Bearer ` prefix.
        """
    def set_bearer_access_token(self, token: str) -> None:
        """Sets one generic bearer token for clients built from this configuration.

        Args:
            token: The token, without the `Bearer ` prefix.
        """
    def clear_auth(self) -> None:
        """Removes every credential source, the token provider included."""
    def get_extra(self) -> _JSONObject:
        """Gives the keys of the configuration file section that the SDK doesn't use.

        The values are as they are in the file, so don't log them if they can hold
        credentials.

        Returns:
            A new `dict` of the section's other keys; empty if the configuration
            wasn't read from a file.
        """
    def set_timeouts(
        self,
        *,
        connect: float | None = None,
        request: float | None = None,
        discovery: float | None = None,
        submission: float | None = None,
    ) -> None:
        """Sets time limits of requests, in seconds.

        Limits that aren't given stay as they are; if one value is invalid, none changes.
        Clients and `Transport` read the limits when they are built, so set them first.

        Args:
            connect: Opening each connection (default 10).
            request: Each request that the other limits don't apply to (default 60).
            discovery: The service check when a client is built, and
                `TRS.service_info()` (default 10).
            submission: `TES.create()`, `AsyncTES.create()` and `WES.run()`, uploads
                included (default 300).

        Raises:
            ValueError: A limit is not a valid number of seconds.
        """
    def get_timeouts(self) -> dict[str, float]:
        """Gives the time limits of requests.

        Returns:
            The limits in seconds, by name: `connect`, `request`, `discovery` and
            `submission`.
        """
    def set_token_provider(
        self,
        provider: Callable[[str], str | tuple[str, float]] | None,
        *,
        audience: str | None = None,
    ) -> None:
        """Gets a bearer token from `provider` for each request, in place of a static one.

        Clients and `Transport` take the provider when they are built. After
        `401 Unauthorized`, the SDK drops a reused token and asks the provider again; a
        `GET` request is sent once more, submissions never. If the provider raises or
        returns something else, the call raises `RuntimeError` and its request isn't sent.
        Set the provider after `from_file()`, which replaces all settings.

        Args:
            provider: Called as `provider(audience)`; returns the token as a `str`, or a
                `(str, float)` tuple with the seconds for which the SDK may reuse it.
                `None` removes the provider.
            audience: The value passed to `provider`; by default, the API root.

        Raises:
            TypeError: `provider` is not callable.
            ValueError: `audience` is given without a provider.
        """
    def from_file(self, service_type: ServiceType | str) -> None:
        """Reads a section of `~/.ga4gh-cli/config.json`.

        It replaces all settings, including a token provider. In a file with profiles, the
        section comes from its default profile. A section can name an environment variable
        for its token with `oauth_access_token_env` or `bearer_access_token_env`.

        Args:
            service_type: A `ServiceType`, or the name of any other section.

        Raises:
            FileNotFoundError: The file doesn't exist.
            ValueError: The section is missing, the file is malformed or a named
                environment variable is unset.
        """

class ServiceInfo:
    """Reads a service's service info."""

    def __init__(self, py_config: Configuration) -> None:
        """Builds a reader for the service of `py_config`; sends no request.

        Args:
            py_config: The service's configuration.
        """
    def get_service_info(self) -> str:
        """Reads the service info.

        Returns:
            The service info as formatted text (`"Service: ..."`), not as a typed model.

        Raises:
            RuntimeError: The request failed, for example because of conflicting
                credentials, or the response is not a service info.
        """

class DrsObject:
    """A DRS object's metadata, as returned by the server.

    Keeps the object's full JSON, including fields that the DRS models don't know.
    Nested values, such as `access_methods`, come as `dict`s and `list`s.
    """

    @property
    def id(self) -> str:
        """The object's ID."""
    @property
    def name(self) -> str | None:
        """The object's name, such as a file name."""
    @property
    def self_uri(self) -> str:
        """The object's `drs://` URI."""
    @property
    def size(self) -> int:
        """The object's size in bytes; for a bundle, the sum of its contents."""
    @property
    def created_time(self) -> str:
        """When the object was created, as an RFC 3339 timestamp."""
    @property
    def updated_time(self) -> str | None:
        """When the object was last updated, as an RFC 3339 timestamp."""
    @property
    def version(self) -> str | None:
        """The object's version, as the server names it."""
    @property
    def mime_type(self) -> str | None:
        """The object's media type, such as `"application/json"`."""
    @property
    def checksums(self) -> list[_JSONObject] | None:
        """The object's checksums, as `dict`s with `type` and `checksum`."""
    @property
    def access_methods(self) -> list[_JSONObject] | None:
        """The ways to fetch the object's bytes, as `dict`s with `type` and `access_id` or
        `access_url`; `None` for a bundle."""
    @property
    def contents(self) -> list[_JSONObject] | None:
        """For a bundle, the objects that it contains, as `dict`s."""
    @property
    def description(self) -> str | None:
        """A description of the object."""
    @property
    def aliases(self) -> list[str] | None:
        """Other names of the object."""
    def json(self) -> str:
        """Gives the object as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the object as a new `dict`, as received."""

class AccessURL:
    """An access URL of a DRS object.

    The URL and headers may hold credentials, such as a signature or a bearer token, so
    `repr()` doesn't show them; don't log them either.
    """

    @property
    def url(self) -> str:
        """The URL to fetch the object's bytes from."""
    @property
    def headers(self) -> list[str] | None:
        """The headers to send with a request to `url`, as `"Name: value"` strings."""
    def json(self) -> str:
        """Gives the access URL as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the access URL as a new `dict`, as received."""

class BulkAccessURL:
    """An access URL from `DRS.get_bulk_access_urls()`, with the IDs it belongs to.

    Like `AccessURL`, `repr()` doesn't show the URL or headers.
    """

    @property
    def drs_object_id(self) -> str | None:
        """The ID of the object that the URL belongs to."""
    @property
    def drs_access_id(self) -> str | None:
        """The access ID that the URL belongs to."""
    @property
    def url(self) -> str:
        """The URL to fetch the object's bytes from."""
    @property
    def headers(self) -> list[str] | None:
        """The headers to send with a request to `url`, as `"Name: value"` strings."""
    def json(self) -> str:
        """Gives the entry as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the entry as a new `dict`, as received."""

class PendingResolution:
    """The server is still preparing an answer: ask again after `retry_after` seconds.

    `api_root` and `endpoint` say what to ask for; they hold no credentials.
    """

    @property
    def api_root(self) -> str:
        """The API root of the service, without credentials, query or fragment."""
    @property
    def endpoint(self) -> str:
        """The endpoint to ask again; for a bulk call, call the bulk method again."""
    @property
    def retry_after(self) -> int:
        """The seconds to wait before asking again (at least 1)."""
    def to_dict(self) -> _JSONObject:
        """Gives `api_root`, `endpoint` and `retry_after` as a new `dict`."""

class BulkPolicy:
    """Limits of the bulk calls of `DRS`.

    Negative counts and negative or non-finite delays raise `ValueError` when the
    policy is built; the other limits are checked when a call starts.
    """

    def __init__(
        self,
        chunk_size: int = 100,
        advertised_limit: int | None = None,
        concurrency: int = 4,
        max_requests: int = 1000,
        max_split_depth: int = 8,
        max_attempts: int = 1,
        initial_delay: float = 0.2,
        max_delay: float = 10.0,
    ) -> None:
        """Builds a policy.

        Negative counts and negative or non-finite delays raise `ValueError` here;
        other invalid values, such as a `chunk_size` of 0, raise it when a bulk
        call starts.

        Args:
            chunk_size: The most IDs per request.
            advertised_limit: The service's most IDs per request, if known; it lowers
                `chunk_size`.
            concurrency: The most requests at the same time.
            max_requests: The most requests in one call, retries and splits included.
            max_split_depth: How often a request that the server rejects as too large
                (413) is halved.
            max_attempts: The most attempts per request, for network errors, 429 and 503.
            initial_delay: The delay before the first retry, in seconds; it doubles up to
                `max_delay`.
            max_delay: The longest delay between retries, in seconds.
        """
    def with_service_info(self, service_info: Mapping[str, Any] | str) -> BulkPolicy:
        """Gives a copy that also keeps to the service's `maxBulkRequestLength`.

        Args:
            service_info: The service info, as a `dict` or JSON text, for example
                `json.loads(drs.transport.get("service-info"))`.

        Returns:
            A new policy, with the same limits if the service info has no
            `maxBulkRequestLength`.

        Raises:
            ValueError: `service_info` isn't JSON, or its `maxBulkRequestLength` is
                invalid.
        """
    @property
    def chunk_size(self) -> int:
        """The most IDs per request."""
    @property
    def advertised_limit(self) -> int | None:
        """The service's most IDs per request, if known; it lowers `chunk_size`."""
    @property
    def concurrency(self) -> int:
        """The most requests at the same time."""
    @property
    def max_requests(self) -> int:
        """The most requests in one call, retries and splits included."""
    @property
    def max_split_depth(self) -> int:
        """How often a request that the server rejects as too large (413) is halved."""
    @property
    def max_attempts(self) -> int:
        """The most attempts per request, for network errors, 429 and 503."""
    @property
    def initial_delay(self) -> float:
        """The delay before the first retry, in seconds; it doubles up to `max_delay`."""
    @property
    def max_delay(self) -> float:
        """The longest delay between retries, in seconds."""

_Resolved = TypeVar("_Resolved", DrsObject, BulkAccessURL, _JSONObject)
"""What a bulk call resolves: a typed result or the complete JSON object."""

class BulkOutcome(Generic[_Resolved]):
    """The outcome for one requested ID (or ID pair) of a bulk call.

    `repr()` shows counts only, as resolved access URLs and error texts may hold
    credentials.
    """

    @property
    def object_id(self) -> str:
        """The requested object ID."""
    @property
    def access_id(self) -> str | None:
        """The access ID of the pair, for `get_bulk_access_urls()`; else `None`."""
    @property
    def resolved(self) -> list[_Resolved]:
        """The results that the server returned for the ID; one, if all went well."""
    @property
    def failure_codes(self) -> list[int]:
        """The HTTP status codes that the server gave for the ID, such as 404."""
    @property
    def pending(self) -> PendingResolution | None:
        """The pending state, if the server is still preparing the answer."""
    @property
    def errors(self) -> list[str]:
        """What went wrong otherwise, such as a missing or duplicate answer."""
    @property
    def is_success(self) -> bool:
        """Whether the server returned exactly one result and nothing else for the ID."""

class BulkReport(Generic[_Resolved]):
    """The outcomes, unattributed problems, and observed reply shapes of a bulk call."""

    @property
    def outcomes(self) -> list[BulkOutcome[_Resolved]]:
        """One outcome per distinct requested ID (or ID pair), sorted by ID."""
    @property
    def diagnostics(self) -> list[str]:
        """Problems with the server's answers that belong to no ID."""
    @property
    def dialects(self) -> list[str]:
        """Reply shapes seen across the call: `standard` and, for objects, `gen3`."""

class DRS:
    """A DRS client (DRS 1.4) for object metadata and access URLs.

    It reads metadata and access URLs, one at a time or in bulk, and never downloads
    object bytes. IDs are sent as one URL path segment each. Subclass it to add
    endpoints, using `self.transport`.
    """

    def __init__(self, config: Configuration) -> None:
        """Builds a client and checks the service.

        Args:
            config: The service's configuration.

        Raises:
            RuntimeError: The service isn't reachable or isn't a DRS.
        """
    @property
    def config(self) -> Configuration:
        """The client's configuration."""
    @property
    def transport(self) -> Transport:
        """A transport with the client's configuration, for further endpoints."""
    def get_object(self, object_id: str) -> DrsObject:
        """Gets an object's metadata.

        Args:
            object_id: The object's ID.

        Returns:
            The object.

        Raises:
            ValueError: `object_id` is empty, `.` or `..`.
            RuntimeError: The request failed, the response doesn't fit the DRS models, or
                the server is still preparing the object; use `get_object_once()` or
                `resolve_object()` for that case.
        """
    def get_object_raw(self, object_id: str) -> _JSONObject:
        """Gets an object's complete JSON, without checking it against the DRS models.

        This also accepts provider extensions such as `created_time: null`.

        Args:
            object_id: The object's ID.

        Returns:
            The object's JSON as a `dict`.

        Raises:
            ValueError: `object_id` is empty, `.` or `..`.
            RuntimeError: The request failed or the server is still preparing the object.
        """
    def get_object_once(self, object_id: str) -> DrsObject | PendingResolution:
        """Asks once for an object; a pending answer (`202 Accepted`) is not an error.

        Args:
            object_id: The object's ID.

        Returns:
            The object, or a `PendingResolution` if the server is still preparing it.

        Raises:
            ValueError: `object_id` is empty, `.` or `..`.
            RuntimeError: The request failed or the response doesn't fit the DRS models.
        """
    def resolve_object(
        self, object_id: str, timeout: float = 60.0, max_attempts: int = 5
    ) -> DrsObject:
        """Gets an object, waiting as the server asks while it prepares the object.

        The wait blocks without the GIL; Ctrl-C takes effect after it returns.

        Args:
            object_id: The object's ID.
            timeout: The most seconds for all requests and waits.
            max_attempts: The most requests.

        Returns:
            The object.

        Raises:
            ValueError: `object_id`, `timeout` or `max_attempts` is invalid.
            RuntimeError: A request failed, or the object wasn't ready within `timeout`
                seconds or `max_attempts` requests.
        """
    def get_access_url(self, object_id: str, access_id: str) -> AccessURL:
        """Gets an access URL of an object.

        Args:
            object_id: The object's ID.
            access_id: The `access_id` of one of the object's `access_methods`.

        Returns:
            The access URL.

        Raises:
            ValueError: An ID is empty, `.` or `..`.
            RuntimeError: The request failed, the response doesn't fit the DRS models, or
                the server is still preparing the URL; use `get_access_url_once()` or
                `resolve_access_url()` for that case.
        """
    def get_access_url_raw(self, object_id: str, access_id: str) -> _JSONObject:
        """Gets an access URL's complete JSON, without checking it against the DRS models.

        Args:
            object_id: The object's ID.
            access_id: The `access_id` of one of the object's `access_methods`.

        Returns:
            The access URL's JSON as a `dict`.

        Raises:
            ValueError: An ID is empty, `.` or `..`.
            RuntimeError: The request failed or the server is still preparing the URL.
        """
    def get_access_url_once(
        self, object_id: str, access_id: str
    ) -> AccessURL | PendingResolution:
        """Asks once for an access URL; a pending answer is not an error.

        Args:
            object_id: The object's ID.
            access_id: The `access_id` of one of the object's `access_methods`.

        Returns:
            The access URL, or a `PendingResolution` if the server is still preparing it.

        Raises:
            ValueError: An ID is empty, `.` or `..`.
            RuntimeError: The request failed or the response doesn't fit the DRS models.
        """
    def resolve_access_url(
        self, object_id: str, access_id: str, timeout: float = 60.0, max_attempts: int = 5
    ) -> AccessURL:
        """Gets an access URL, waiting as the server asks while it prepares the URL.

        Args:
            object_id: The object's ID.
            access_id: The `access_id` of one of the object's `access_methods`.
            timeout: The most seconds for all requests and waits.
            max_attempts: The most requests.

        Returns:
            The access URL.

        Raises:
            ValueError: An ID, `timeout` or `max_attempts` is invalid.
            RuntimeError: A request failed, or the URL wasn't ready within `timeout`
                seconds or `max_attempts` requests.
        """
    def get_bulk_objects(
        self,
        ids: Iterable[str],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[DrsObject]:
        """Gets the metadata of several objects with `POST objects`.

        Failures of single IDs and of requests are reported in the outcomes, not raised.

        Args:
            ids: The object IDs; repeated IDs are asked for once.
            passports: GA4GH Passport tokens to send in the request body.
            policy: Limits of the call; by default `BulkPolicy()`.
            timeout: The most seconds for the whole call.

        Returns:
            A report with one outcome per distinct ID, holding `DrsObject`s.

        Raises:
            ValueError: An ID, the policy or `timeout` is invalid. An invalid
                policy is rejected even without IDs.
        """
    def get_bulk_objects_raw(
        self,
        ids: Iterable[str],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[_JSONObject]:
        """Like `get_bulk_objects()`, but each result is the complete JSON `dict`.

        Use it for objects that don't fit the DRS models, such as Gen3 objects with
        `created_time: null`.

        Args:
            ids: The object IDs; repeated IDs are asked for once.
            passports: GA4GH Passport tokens to send in the request body.
            policy: Limits of the call; by default `BulkPolicy()`.
            timeout: The most seconds for the whole call.

        Returns:
            A report with one outcome per distinct ID, holding `dict`s.

        Raises:
            ValueError: An ID, the policy or `timeout` is invalid.
        """
    def get_bulk_access_urls(
        self,
        pairs: Mapping[str, str] | Iterable[tuple[str, str]],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[BulkAccessURL]:
        """Gets access URLs of several objects with `POST objects/access`.

        Failures of single pairs and of requests are reported in the outcomes, not raised.

        Args:
            pairs: Object IDs mapped to access IDs, or `(object_id, access_id)` pairs;
                one access ID per object per call.
            passports: GA4GH Passport tokens to send in the request body.
            policy: Limits of the call; by default `BulkPolicy()`.
            timeout: The most seconds for the whole call.

        Returns:
            A report with one outcome per distinct pair, holding `BulkAccessURL`s.

        Raises:
            ValueError: A pair, the policy or `timeout` is invalid.
        """
    def get_bulk_access_urls_raw(
        self,
        pairs: Mapping[str, str] | Iterable[tuple[str, str]],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[_JSONObject]:
        """Like `get_bulk_access_urls()`, but each result is the complete JSON `dict`.

        Args:
            pairs: Object IDs mapped to access IDs, or `(object_id, access_id)` pairs;
                one access ID per object per call.
            passports: GA4GH Passport tokens to send in the request body.
            policy: Limits of the call; by default `BulkPolicy()`.
            timeout: The most seconds for the whole call.

        Returns:
            A report with one outcome per distinct pair, holding `dict`s.

        Raises:
            ValueError: A pair, the policy or `timeout` is invalid.
        """

class DrsUri:
    """A hostname-based DRS URI, such as `drs://drs.example.org/object-id`."""

    def __init__(self, uri: str) -> None:
        """Parses a DRS URI.

        Args:
            uri: The URI, `drs://<host>/<object-id>`. The host is a name or an
                IPv4 address, without a port. IPv6 literals are unsupported.

        Raises:
            ValueError: The URI is malformed, has a port or an IPv6 literal,
                or is a compact identifier (`drs://prefix:accession`), which
                is not supported.
        """
    @property
    def authority(self) -> str:
        """The normalized host: lowercase, punycode, or dotted decimal IPv4."""
    @property
    def object_id(self) -> str:
        """The object ID, decoded."""
    def default_api_root(self) -> str:
        """Gives the API root that the URI implies, `https://<authority>/ga4gh/drs/v1`."""

class AccessPlan:
    """How to fetch a DRS object's bytes: an access method, its URL and its headers.

    The URL and headers may hold credentials, so `repr()` shows only the URL's origin.
    """

    @property
    def object_id(self) -> str:
        """The ID of the object."""
    @property
    def method(self) -> str:
        """The type of the access method, such as `"https"` or `"s3"`."""
    @property
    def region(self) -> str | None:
        """The region of the access method, such as `"us-east-1"`."""
    @property
    def url(self) -> str:
        """The URL to fetch the object's bytes from."""
    @property
    def headers(self) -> list[tuple[str, str]]:
        """`(name, value)` pairs, unlike the `"Name: value"` strings of `AccessURL`."""

class DrsResolver:
    """Turns hostname-based DRS URIs into `AccessPlan`s.

    By default, a URI's host is asked anonymously at `https://<host>/ga4gh/drs/v1`.
    `with_service()` maps a host to a `Transport` with its own API root and settings;
    settings are never sent to other hosts. It never downloads object bytes.
    """

    def __init__(self) -> None:
        """Builds a resolver that knows no services yet."""
    def with_service(self, authority: str, transport: Transport) -> Self:
        """Asks the DRS service of `transport` for URIs with host `authority`.

        Add services before sharing the resolver between threads.

        Args:
            authority: The host of the URIs, without a port.
            transport: A transport with the service's API root and settings.

        Returns:
            The resolver itself, for chaining.

        Raises:
            ValueError: `authority` is not a valid host, has a port or is an
                IPv6 literal.
            RuntimeError: Another thread is planning access with the resolver.
        """
    def access_plan_once(
        self,
        uri: str,
        protocols: Sequence[str] | None = None,
        regions: Sequence[str] | None = None,
        method_index: int | None = None,
        timeout: float = 60.0,
    ) -> AccessPlan | PendingResolution:
        """Asks once for the object of `uri` and, if needed, its access URL.

        Args:
            uri: A hostname-based DRS URI.
            protocols: The access method types to choose from, such as `["https", "s3"]`,
                in order of preference.
            regions: The regions to choose from, in order of preference.
            method_index: The index of the access method to use, instead of choosing.
            timeout: The most seconds for all requests.

        Returns:
            The plan, or a `PendingResolution` if the server is still preparing an answer.

        Raises:
            ValueError: The URI or an argument is invalid.
            RuntimeError: A request failed or the object has no usable access method.
        """
    def access_plan(
        self,
        uri: str,
        protocols: Sequence[str] | None = None,
        regions: Sequence[str] | None = None,
        method_index: int | None = None,
        timeout: float = 60.0,
        max_attempts: int = 5,
    ) -> AccessPlan:
        """Plans access to the object of `uri`, waiting while the server prepares answers.

        Reads the object, picks an available access method and reads its access URL if the
        method has none.

        Args:
            uri: A hostname-based DRS URI.
            protocols: The access method types to choose from, in order of preference.
            regions: The regions to choose from, in order of preference.
            method_index: The index of the access method to use, instead of choosing.
            timeout: The most seconds for all requests and waits.
            max_attempts: The most requests for each answer.

        Returns:
            The plan.

        Raises:
            ValueError: The URI or an argument is invalid.
            RuntimeError: A request failed, an answer wasn't ready in time, or the object
                has no usable access method.
        """

class Transport:
    """Sends requests below the configured API root and returns response bodies as text.

    Subclass it to add helpers for further endpoints. In all methods, `endpoint` is
    relative to the API root, such as `tasks/123`; encode IDs that you put into it, for
    example with `urllib.parse.quote(task_id, safe="")`. `params` holds query parameters,
    as a mapping or a JSON object string: values are strings, numbers, booleans or lists
    of these, a list repeats the parameter, and `None` or an empty list leaves it out.
    """

    def __init__(self, py_config: Configuration) -> None:
        """Builds a transport with the API root and settings of `py_config`.

        Conflicting credentials raise `RuntimeError` on the first request.

        Args:
            py_config: The service's configuration.
        """
    def get(self, endpoint: str, params: _Params | None = None) -> str:
        """Sends a `GET` request.

        Args:
            endpoint: The path below the API root.
            params: Query parameters.

        Returns:
            The response body.

        Raises:
            ValueError: `params` holds an unsupported value.
            RuntimeError: The request failed or the server answered with an error status.
        """
    def post(self, endpoint: str, data: Any = None, params: _Params | None = None) -> str:
        """Sends a `POST` request with a JSON body.

        Args:
            endpoint: The path below the API root.
            data: Any object that `json.dumps` accepts; a `str` is taken as JSON text,
                so pass `json.dumps(text)` to send a JSON string.
            params: Query parameters.

        Returns:
            The response body.

        Raises:
            ValueError: `data` or `params` is invalid.
            RuntimeError: The request failed or the server answered with an error status.
        """
    def put(self, endpoint: str, data: Any, params: _Params | None = None) -> str:
        """Sends a `PUT` request with a JSON body.

        Args:
            endpoint: The path below the API root.
            data: Any object that `json.dumps` accepts; a `str` is taken as JSON text.
            params: Query parameters.

        Returns:
            The response body.

        Raises:
            ValueError: `data` or `params` is invalid.
            RuntimeError: The request failed or the server answered with an error status.
        """
    def delete(self, endpoint: str, params: _Params | None = None) -> str:
        """Sends a `DELETE` request.

        Args:
            endpoint: The path below the API root.
            params: Query parameters.

        Returns:
            The response body.

        Raises:
            ValueError: `params` holds an unsupported value.
            RuntimeError: The request failed or the server answered with an error status.
        """
    def post_multipart(
        self,
        endpoint: str,
        fields: list[tuple[str, str]] | None = None,
        files: list[tuple[str, str, bytes] | tuple[str, str, bytes, str]] | None = None,
        params: _Params | None = None,
    ) -> str:
        """Sends a `multipart/form-data` `POST` request.

        Names may repeat, as for WES `workflow_attachment`. To start a WES run, use
        `WES.run()` instead.

        Args:
            endpoint: The path below the API root.
            fields: `(name, value)` text parts.
            files: `(name, filename, content)` or `(name, filename, content, content_type)`
                file parts, with `content` as `bytes`.
            params: Query parameters.

        Returns:
            The response body.

        Raises:
            TypeError: An argument has the wrong type, such as a `dict` for `fields`.
            ValueError: A value is invalid.
            RuntimeError: The request failed or the server answered with an error status.
        """

class ToolVersion:
    """A tool version, as returned by a TRS, with its JSON.

    `json()` and `to_dict()` return the version as JSON, including fields that the TRS
    models don't know.
    """

    @property
    def id(self) -> str:
        """The version's ID."""
    @property
    def url(self) -> str:
        """The version's URL in the registry."""
    @property
    def name(self) -> str | None:
        """The version's name, such as a tag; some registries take it as its ID."""
    @property
    def author(self) -> list[str] | None:
        """The version's authors."""
    @property
    def description(self) -> str | None:
        """The version's description (TRS 2.0.2), if it is a string."""
    @property
    def is_production(self) -> bool | None:
        """Whether the version is meant for production use."""
    @property
    def images(self) -> list[_JSONObject] | None:
        """The container images of the version, as `dict`s."""
    @property
    def descriptor_type(self) -> list[str] | None:
        """The descriptor types of the version, such as `["CWL"]`."""
    @property
    def descriptor_type_version(self) -> dict[str, list[str]] | None:
        """The language versions of the descriptors, by descriptor type."""
    @property
    def containerfile(self) -> bool | None:
        """Whether the version has container files, such as a Dockerfile."""
    @property
    def meta_version(self) -> str | None:
        """The version of the version's metadata in the registry."""
    @property
    def verified(self) -> bool | None:
        """Whether the version is verified."""
    @property
    def verified_source(self) -> list[str] | None:
        """Who verified the version."""
    @property
    def signed(self) -> bool | None:
        """Whether the version is signed."""
    @property
    def included_apps(self) -> list[str] | None:
        """Tools that the version includes, as TRS URIs or URLs."""
    def json(self) -> str:
        """Gives the version as JSON text."""
    def to_dict(self) -> _JSONObject:
        """Gives the version as a new `dict`."""

class Tool:
    """A tool with its versions, as returned by a TRS, with its JSON.

    `json()` and `to_dict()` return the tool as JSON, including fields that the TRS
    models don't know.
    """

    @property
    def id(self) -> str:
        """The tool's ID."""
    @property
    def url(self) -> str:
        """The tool's URL in the registry."""
    @property
    def aliases(self) -> list[str] | None:
        """Other IDs of the tool, such as DOIs."""
    @property
    def organization(self) -> str:
        """The organization that publishes the tool."""
    @property
    def name(self) -> str | None:
        """The tool's name."""
    @property
    def toolclass(self) -> _JSONObject | None:
        """The tool class as a `dict`, such as `{"id": "1", "name": "Workflow"}`."""
    @property
    def description(self) -> str | None:
        """The tool's description."""
    @property
    def meta_version(self) -> str | None:
        """The version of the tool's metadata in the registry."""
    @property
    def has_checker(self) -> bool | None:
        """Whether the tool has a checker tool."""
    @property
    def checker_url(self) -> str | None:
        """The URL of the tool's checker tool."""
    @property
    def versions(self) -> list[ToolVersion]:
        """The tool's versions."""
    def json(self) -> str:
        """Gives the tool as JSON text."""
    def to_dict(self) -> _JSONObject:
        """Gives the tool as a new `dict`."""

class FileWrapper:
    """A descriptor, test or container file: its `content`, or a `url` to fetch it from.

    For `PLAIN_*` descriptor types, only `content` is set. `repr()` shows the length of
    `content`, not the content.
    """

    @property
    def content(self) -> str | None:
        """The file's content."""
    @property
    def url(self) -> str | None:
        """The URL to fetch the file from."""
    @property
    def checksum(self) -> list[_JSONObject] | None:
        """The file's checksums, as `dict`s with `type` and `checksum`."""
    @property
    def image_type(self) -> str | None:
        """The image or descriptor type of the file, such as `"Docker"` or `"CWL"`."""
    def json(self) -> str:
        """Gives the file as JSON text."""
    def to_dict(self) -> _JSONObject:
        """Gives the file as a new `dict`."""

class ToolFile:
    """A file of a tool version, from `TRS.list_files()`."""

    @property
    def path(self) -> str | None:
        """The file's path, relative to the primary descriptor."""
    @property
    def file_type(self) -> str | None:
        """The file type, such as `"PRIMARY_DESCRIPTOR"`, as the server sent it."""
    @property
    def checksum(self) -> _JSONObject | None:
        """The file's checksum, as a `dict` with `type` and `checksum`."""
    def json(self) -> str:
        """Gives the file as JSON text."""
    def to_dict(self) -> _JSONObject:
        """Gives the file as a new `dict`."""

class ToolPage:
    """One page of tools from `TRS.list_tools()` or `TRS.next_tools_page()`.

    The paging properties hold the TRS paging headers, as `str`s, or `None` if the server
    didn't send them; a repeated header gives its first value. `len(page)` is the number
    of tools.
    """

    @property
    def tools(self) -> list[Tool]:
        """The tools of the page."""
    @property
    def next_page(self) -> str | None:
        """The URL of the next page (`next_page` header); `None` on the last page."""
    @property
    def last_page(self) -> str | None:
        """The URL of the last page (`last_page` header)."""
    @property
    def self_link(self) -> str | None:
        """The URL of this page (`self_link` header)."""
    @property
    def current_offset(self) -> str | None:
        """The offset of this page (`current_offset` header), as the server sent it."""
    @property
    def current_limit(self) -> str | None:
        """The page size (`current_limit` header), as the server sent it."""
    def headers(self) -> dict[str, str]:
        """All response headers by lowercase name; repeated ones joined with `", "`.

        They can hold cookies, so don't log them.
        """
    def __len__(self) -> int:
        """Gives the number of tools on the page."""

class TRS:
    """A TRS client (TRS 2.0.1 and 2.0.2).

    `api_version` is `"auto"`, `"strict"`, `"2.0.1"` or `"2.0.2"`; `tag_filter` is
    `"auto"`, `"enabled"` or `"disabled"`. `discovery=False` skips the service info and
    needs an explicit `api_version`.

    IDs, version IDs and relative paths are sent unchanged, as one percent-encoded URL path
    segment each, so IDs such as Dockstore's `#workflow/github.com/org/repo` work. Empty
    values, `.` and `..` raise `ValueError` before a request. Failed requests, such as for
    a missing tool (404), and responses that don't fit the TRS models raise `RuntimeError`.
    """

    def __init__(
        self,
        config: Configuration,
        *,
        api_version: str = "auto",
        tag_filter: str = "auto",
        discovery: bool = True,
    ) -> None:
        """Builds a client; reads the service info unless `discovery` is `False`.

        Args:
            config: The service's configuration.
            api_version: The TRS version to read responses with: `"auto"` (the declared
                version, else 2.0.1), `"strict"` (only a declared version that the SDK
                supports), `"2.0.1"` or `"2.0.2"`.
            tag_filter: Whether the server supports the `tags` filter of `list_tools()`,
                beyond what its TRS version says: `"auto"`, `"enabled"` or `"disabled"`.
            discovery: Whether to read the service info; `False` needs `api_version`
                `"2.0.1"` or `"2.0.2"` and doesn't check that the service is a TRS.

        Raises:
            ValueError: An option is invalid.
            RuntimeError: The service isn't reachable or isn't a TRS, or, with `"strict"`,
                declares no supported version.
        """
    @property
    def config(self) -> Configuration:
        """The client's configuration."""
    @property
    def transport(self) -> Transport:
        """A transport with the client's configuration, for further endpoints."""
    @property
    def api_version(self) -> str:
        """The TRS version that responses are read with: `"2.0.1"` or `"2.0.2"`."""
    @property
    def declared_version(self) -> str | None:
        """The TRS version that the service info declares, or `None` without discovery."""
    @property
    def version_source(self) -> str:
        """Where `api_version` comes from: `"declared"`, `"caller"` or `"baseline"`.

        `"baseline"` means that the service declared an unknown version, or none, and is
        read as 2.0.1 without a check that this works.
        """
    @property
    def tag_filter(self) -> str:
        """Whether `list_tools(tags=...)` may be sent.

        It may if this is `"specified"` (TRS 2.0.2) or `"caller_enabled"`, and may not if
        it is `"not_specified"`, `"caller_disabled"` or `"unknown"`.
        """
    @property
    def discovered_service_info(self) -> _JSONObject | None:
        """The service info read when the client was built, or `None` without discovery."""
    def service_info(self) -> _JSONObject:
        """Reads the service info again.

        Returns:
            The service info as a `dict`.

        Raises:
            RuntimeError: The request failed.
        """
    def list_tool_classes(self) -> list[_JSONObject]:
        """Lists the registry's tool classes.

        Returns:
            The tool classes as `dict`s, such as `{"id": "1", "name": "Workflow"}`.

        Raises:
            RuntimeError: The request failed.
        """
    def list_tools(
        self,
        *,
        id: str | None = None,
        alias: str | None = None,
        tool_class: str | None = None,
        descriptor_type: str | None = None,
        registry: str | None = None,
        organization: str | None = None,
        name: str | None = None,
        toolname: str | None = None,
        description: str | None = None,
        author: str | None = None,
        checker: bool | None = None,
        tags: Sequence[str] | None = None,
        offset: str | None = None,
        limit: int | None = None,
    ) -> ToolPage:
        """Lists tools, one page at a time; unset filters are not sent.

        Requested filters are never dropped silently.

        Args:
            id: A tool ID.
            alias: An alias of a tool.
            tool_class: A tool class name, such as `"Workflow"`.
            descriptor_type: A descriptor type, such as `"CWL"`.
            registry: The image registry of the tools' containers.
            organization: The publishing organization.
            name: The image name.
            toolname: The tool's name.
            description: Text in the tool's description.
            author: An author.
            checker: Whether to list checker tools.
            tags: Tags that the tools have (TRS 2.0.2); sent as one comma-separated value.
            offset: Where the page starts, as the server's paging gives it.
            limit: The most tools on the page, between 1 and 2147483647.

        Returns:
            The first or requested page.

        Raises:
            ValueError: `limit` is out of range, `descriptor_type` is empty, `.` or `..`,
                a tag is empty or contains a comma, or `tags` is given while
                `tag_filter` doesn't allow it.
            RuntimeError: The request failed.
        """
    def next_tools_page(self, next_page: str) -> ToolPage:
        """Gets the page at `ToolPage.next_page`.

        Only URLs of the tools endpoint of the client's API root are followed, so
        credentials never go to another server.

        Args:
            next_page: The `next_page` URL of the previous page.

        Returns:
            The next page.

        Raises:
            ValueError: `next_page` is empty.
            RuntimeError: The URL is not at the client's tools endpoint (before a request),
                or the request failed.
        """
    def get_tool(self, id: str) -> Tool:
        """Gets a tool, with its versions.

        Args:
            id: The tool's ID.

        Returns:
            The tool.

        Raises:
            ValueError: `id` is empty, `.` or `..`.
            RuntimeError: The request failed, for example because the tool doesn't exist.
        """
    def list_versions(self, id: str) -> list[ToolVersion]:
        """Lists the versions of a tool.

        Args:
            id: The tool's ID.

        Returns:
            The versions.

        Raises:
            ValueError: `id` is empty, `.` or `..`.
            RuntimeError: The request failed.
        """
    def get_version(self, id: str, version_id: str) -> ToolVersion:
        """Gets one version of a tool.

        Args:
            id: The tool's ID.
            version_id: The version's ID; some registries, such as Dockstore, take its name.

        Returns:
            The version.

        Raises:
            ValueError: An ID is empty, `.` or `..`.
            RuntimeError: The request failed.
        """
    def get_descriptor(
        self,
        id: str,
        version_id: str,
        descriptor_type: str,
        relative_path: str | None = None,
    ) -> FileWrapper:
        """Gets the primary descriptor, or the file at `relative_path` below it.

        Args:
            id: The tool's ID.
            version_id: The version's ID.
            descriptor_type: A TRS descriptor type, such as `"CWL"`, `"WDL"`, `"NFL"`,
                `"GALAXY"` or `"SMK"`; or `"PLAIN_CWL"` and so on for the bare file, which
                comes as a `FileWrapper` with only `content`. Types are sent as given.
            relative_path: The path of another file, relative to the primary descriptor.

        Returns:
            The file.

        Raises:
            ValueError: An ID, `descriptor_type` or `relative_path` is empty, `.` or `..`.
            RuntimeError: The request failed.
        """
    def list_tests(self, id: str, version_id: str, descriptor_type: str) -> list[FileWrapper]:
        """Lists the test parameter files of a tool version.

        Args:
            id: The tool's ID.
            version_id: The version's ID.
            descriptor_type: A descriptor type, such as `"CWL"`; for a `PLAIN_` type, a bare
                test file comes as one `FileWrapper` with only `content`.

        Returns:
            The test files.

        Raises:
            ValueError: An ID or `descriptor_type` is empty, `.` or `..`.
            RuntimeError: The request failed.
        """
    def list_files(self, id: str, version_id: str, descriptor_type: str) -> list[ToolFile]:
        """Lists the files of a tool version, with their paths and file types.

        Args:
            id: The tool's ID.
            version_id: The version's ID.
            descriptor_type: A descriptor type without `PLAIN_`, such as `"CWL"`.

        Returns:
            The files.

        Raises:
            ValueError: An ID or `descriptor_type` is empty, `.` or `..`.
            RuntimeError: The request failed.
        """
    def get_files_zip(
        self, id: str, version_id: str, descriptor_type: str, limit: int | None = None
    ) -> bytes:
        """Downloads the version's files as one ZIP archive of at most `limit` bytes.

        The archive is read into memory; it isn't extracted or checked further.

        Args:
            id: The tool's ID.
            version_id: The version's ID.
            descriptor_type: A descriptor type without `PLAIN_`, such as `"CWL"`.
            limit: The most bytes to read; `None` means 64 MiB (67108864 bytes).

        Returns:
            The archive.

        Raises:
            ValueError: An ID or `descriptor_type` is empty, `.` or `..`, or `limit` is
                below 1.
            RuntimeError: The request failed, the archive is larger than `limit`, or the
                response isn't a ZIP archive.
        """
    def get_containerfiles(self, id: str, version_id: str) -> list[FileWrapper]:
        """Gets the container specifications, such as Dockerfiles, of a tool version.

        Args:
            id: The tool's ID.
            version_id: The version's ID.

        Returns:
            The container files.

        Raises:
            ValueError: An ID is empty, `.` or `..`.
            RuntimeError: The request failed.
        """

class WesState(str, enum.Enum):
    """The state of a WES run. Members are strings equal to the state's name.

    `str()`, f-strings and `json.dumps()` give the state's name; `WesState("RUNNING")`
    looks a state up by name.
    """

    UNKNOWN = "UNKNOWN"
    """The state is unknown, or one that WES 1.1 doesn't define."""
    QUEUED = "QUEUED"
    """The run is queued."""
    INITIALIZING = "INITIALIZING"
    """The run is being prepared."""
    RUNNING = "RUNNING"
    """The run is running."""
    PAUSED = "PAUSED"
    """The run is paused."""
    COMPLETE = "COMPLETE"
    """The run has completed."""
    EXECUTOR_ERROR = "EXECUTOR_ERROR"
    """The run failed because a task failed."""
    SYSTEM_ERROR = "SYSTEM_ERROR"
    """The run failed because of a system error."""
    CANCELED = "CANCELED"
    """The run was canceled."""
    CANCELING = "CANCELING"
    """The run is being canceled."""
    PREEMPTED = "PREEMPTED"
    """The run was preempted."""

    def __str__(self) -> str:
        """Gives the state's name."""
    def __format__(self, format_spec: str) -> str:
        """Formats the state's name, as `str.__format__` does.

        Args:
            format_spec: The format specification, such as `">10"`.

        Returns:
            The formatted name.
        """

class RunRequest:
    """A WES run request: a workflow to run, with its parameters.

    Keeps the request's full JSON, including fields that the WES models don't know.
    `WES.run()` sends those fields as further form parts: strings as they are, other
    values as JSON. Requests can be pickled and copied.
    """

    def __init__(self, request: RunRequest | Mapping[str, Any] | str) -> None:
        """Builds a request and checks it against the WES models.

        Args:
            request: A `RunRequest`, a `dict` (or other JSON-serialisable mapping), or JSON
                text. Unlike for `WES.run()`, a `str` is JSON, not a path.

        Raises:
            ValueError: The request doesn't fit the WES models.
        """
    @classmethod
    def from_dict(cls, request: Mapping[str, Any]) -> Self:
        """Builds a request from a `dict`, as `cls(request)`.

        Args:
            request: The request's JSON as a mapping.

        Returns:
            The request.

        Raises:
            ValueError: The request doesn't fit the WES models.
        """
    @classmethod
    def from_json(cls, text: str) -> Self:
        """Builds a request from JSON text.

        Args:
            text: The request as a JSON object.

        Returns:
            The request.

        Raises:
            ValueError: `text` isn't JSON, or the request doesn't fit the WES models.
        """
    @classmethod
    def from_file(cls, path: str | os.PathLike[str]) -> Self:
        """Reads a request from a JSON file.

        Args:
            path: The file's path.

        Returns:
            The request.

        Raises:
            OSError: The file can't be read, for example `FileNotFoundError`.
            ValueError: The file isn't JSON, or the request doesn't fit the WES models.
        """
    @property
    def workflow_type(self) -> str:
        """The workflow language, such as `"CWL"`."""
    @property
    def workflow_type_version(self) -> str:
        """The version of the workflow language, such as `"v1.2"`."""
    @property
    def workflow_url(self) -> str:
        """The workflow's URL, or the filename of an attachment."""
    @property
    def workflow_params(self) -> _JSONObject | None:
        """The workflow's input parameters."""
    @property
    def tags(self) -> dict[str, str] | None:
        """Key-value tags of the run."""
    @property
    def workflow_engine(self) -> str | None:
        """The workflow engine to use."""
    @property
    def workflow_engine_version(self) -> str | None:
        """The version of the workflow engine."""
    @property
    def workflow_engine_parameters(self) -> dict[str, str] | None:
        """Parameters for the workflow engine."""
    def json(self) -> str:
        """Gives the request as JSON text, as given or received."""
    def to_dict(self) -> _JSONObject:
        """Gives the request as a new `dict`, as given or received."""

class RunLog:
    """A run's details, with its request, state, logs and outputs, as returned by the server.

    Keeps the run's full JSON, including fields that the WES models don't know.
    """

    @property
    def run_id(self) -> str | None:
        """The run's ID."""
    @property
    def request(self) -> RunRequest | None:
        """The run's request, or `None` if it is missing or doesn't fit the WES models."""
    @property
    def state(self) -> WesState:
        """The run's state; `UNKNOWN` if it has none or one that WES 1.1 doesn't define."""
    @property
    def run_log(self) -> _JSONObject | None:
        """The log of the workflow engine, such as its command, times and exit code."""
    @property
    def task_logs_url(self) -> str | None:
        """The URL of the run's task list (WES 1.1)."""
    @property
    def task_logs(self) -> list[_JSONObject] | None:
        """The run's task logs, as `dict`s; WES 1.1 lists them with `Run.tasks()`."""
    @property
    def outputs(self) -> Any:
        """The run's outputs, as JSON values."""
    def json(self) -> str:
        """Gives the run as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the run as a new `dict`, as received."""

class RunSummary:
    """A run in the list of `WES.list()`."""

    @property
    def run_id(self) -> str:
        """The run's ID."""
    @property
    def state(self) -> WesState:
        """The run's state; `UNKNOWN` if it has none or one that WES 1.1 doesn't define."""
    @property
    def start_time(self) -> str | None:
        """When the run started, as an RFC 3339 timestamp."""
    @property
    def end_time(self) -> str | None:
        """When the run ended, as an RFC 3339 timestamp."""
    @property
    def tags(self) -> dict[str, str] | None:
        """Key-value tags of the run."""
    def json(self) -> str:
        """Gives the run as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the run as a new `dict`, as received."""

class RunListResponse:
    """One page of runs from `WES.list()`."""

    @property
    def runs(self) -> list[RunSummary]:
        """The runs of the page."""
    @property
    def next_page_token(self) -> str | None:
        """The token of the next page, for `WES.list(page_token=...)`."""
    def json(self) -> str:
        """Gives the response as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the response as a new `dict`, as received."""

class TaskLog:
    """A task of a run, from `Run.tasks()` or `Run.task()` (WES 1.1)."""

    @property
    def id(self) -> str:
        """The task's ID."""
    @property
    def name(self) -> str:
        """The task's name."""
    @property
    def cmd(self) -> list[str] | None:
        """The command that the task ran."""
    @property
    def start_time(self) -> str | None:
        """When the task started, as an RFC 3339 timestamp."""
    @property
    def end_time(self) -> str | None:
        """When the task ended, as an RFC 3339 timestamp."""
    @property
    def stdout(self) -> str | None:
        """The URL of the task's standard output."""
    @property
    def stderr(self) -> str | None:
        """The URL of the task's standard error."""
    @property
    def exit_code(self) -> int | None:
        """The task's exit code."""
    @property
    def system_logs(self) -> list[str] | None:
        """System messages about the task, such as errors."""
    @property
    def tes_uri(self) -> str | None:
        """The URI of the task in a TES, if it ran on one."""
    def json(self) -> str:
        """Gives the task as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the task as a new `dict`, as received."""

class TaskListResponse:
    """One page of tasks from `Run.tasks()`."""

    @property
    def task_logs(self) -> list[TaskLog]:
        """The tasks of the page."""
    @property
    def next_page_token(self) -> str | None:
        """The token of the next page, for `Run.tasks(page_token=...)`."""
    def json(self) -> str:
        """Gives the response as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the response as a new `dict`, as received."""

class Run:
    """A WES run on the server, addressed by its ID."""

    def __init__(self, id: str, transport: Transport) -> None:
        """Addresses an existing run; sends no request.

        Args:
            id: The run's ID.
            transport: A transport with the WES service's configuration, such as
                `wes.transport`.

        Raises:
            ValueError: `id` is empty.
        """
    @property
    def id(self) -> str:
        """The run's ID."""
    @property
    def transport(self) -> Transport:
        """A transport with the run's configuration, for further endpoints."""
    def status(self) -> WesState:
        """Gets the run's current state.

        Returns:
            The state.

        Raises:
            RuntimeError: The request failed.
        """
    def cancel(self) -> None:
        """Asks the server to cancel the run.

        Raises:
            RuntimeError: The request failed.
        """
    def log(self) -> RunLog:
        """Gets the run's details, with its request, state, logs and outputs.

        Returns:
            The run's details.

        Raises:
            RuntimeError: The request failed or the response doesn't fit the WES models.
        """
    def tasks(
        self, page_size: int | None = None, page_token: str | None = None
    ) -> TaskListResponse:
        """Lists the run's tasks, one page at a time (WES 1.1).

        Args:
            page_size: The most tasks on the page.
            page_token: The `next_page_token` of the previous page.

        Returns:
            The page.

        Raises:
            RuntimeError: The request failed or the response doesn't fit the WES models.
        """
    def task(self, task_id: str) -> TaskLog:
        """Gets one of the run's tasks (WES 1.1).

        Args:
            task_id: The task's ID.

        Returns:
            The task.

        Raises:
            ValueError: `task_id` is empty, `.` or `..`.
            RuntimeError: The request failed or the response doesn't fit the WES models.
        """

_Attachments: TypeAlias = (
    Mapping[str, bytes | bytearray | os.PathLike[str]]
    | Iterable[tuple[str, bytes | bytearray | os.PathLike[str]]]
)
"""Workflow attachments: filenames (relative paths) mapped to contents or file paths."""

class WES:
    """A WES client (WES 1.1). Subclass it to add endpoints, using `self.transport`."""

    def __init__(self, config: Configuration) -> None:
        """Builds a client and checks the service.

        Args:
            config: The service's configuration.

        Raises:
            RuntimeError: The service isn't reachable or isn't a WES.
        """
    @property
    def config(self) -> Configuration:
        """The client's configuration."""
    @property
    def transport(self) -> Transport:
        """A transport with the client's configuration, for further endpoints."""
    def run(
        self,
        request: RunRequest | Mapping[str, Any] | str | os.PathLike[str],
        attachments: _Attachments | None = None,
    ) -> Run:
        """Starts a workflow run.

        `workflow_type`, `workflow_type_version`, `workflow_url` and `workflow_params` are
        required. Request fields that the WES models don't know are sent as further form
        parts. The upload and the answer must finish within the `submission` time limit.
        If starting fails after submission, the server may have started the run; don't
        retry automatically.

        Args:
            request: A `RunRequest`, a `dict` (or other mapping), or the path of a JSON
                file as `str` or `os.PathLike`. A `str` is always a path; pass JSON text
                as `RunRequest.from_json(text)`.
            attachments: Filenames (relative paths with `/`, such as `tools/echo.cwl`)
                mapped to contents as `bytes` or to file paths (`os.PathLike`), which are
                streamed; or a list of `(filename, content)` pairs.

        Returns:
            The run.

        Raises:
            ValueError: The request or an attachment name is invalid, a `str` starts with
                `{` (after whitespace), or the request has a `workflow_attachment` field
                (pass files with `attachments`).
            OSError: A request or attachment file can't be read.
            RuntimeError: The request failed or the response doesn't fit the WES models.
        """
    def list(
        self, page_size: int | None = None, page_token: str | None = None
    ) -> RunListResponse:
        """Lists runs, one page at a time.

        A bare array of runs, as WES 0.2.0 servers send it, becomes a page without a token.

        Args:
            page_size: The most runs on the page.
            page_token: The `next_page_token` of the previous page.

        Returns:
            The page.

        Raises:
            RuntimeError: The request failed or the response doesn't fit the WES models.
        """
    def get(self, id: str) -> RunLog:
        """Gets a run's details, like `Run(id, wes.transport).log()`.

        Args:
            id: The run's ID.

        Returns:
            The run's details.

        Raises:
            ValueError: `id` is empty, `.` or `..`.
            RuntimeError: The request failed or the response doesn't fit the WES models.
        """

class TesState(str, enum.Enum):
    """The state of a TES task. Members are strings equal to the state's name.

    `str()`, f-strings and `json.dumps()` give the state's name; `TesState("RUNNING")`
    looks a state up by name.
    """

    UNKNOWN = "UNKNOWN"
    """The state is unknown, or one that TES 1.1 doesn't define."""
    QUEUED = "QUEUED"
    """The task is queued."""
    INITIALIZING = "INITIALIZING"
    """The task is being prepared, for example its inputs downloaded."""
    RUNNING = "RUNNING"
    """The task is running."""
    PAUSED = "PAUSED"
    """The task is paused."""
    COMPLETE = "COMPLETE"
    """The task has completed."""
    EXECUTOR_ERROR = "EXECUTOR_ERROR"
    """The task failed because an executor failed."""
    SYSTEM_ERROR = "SYSTEM_ERROR"
    """The task failed because of a system error."""
    CANCELED = "CANCELED"
    """The task was canceled."""
    PREEMPTED = "PREEMPTED"
    """The task was preempted."""
    CANCELING = "CANCELING"
    """The task is being canceled."""

    def __str__(self) -> str:
        """Gives the state's name."""
    def __format__(self, format_spec: str) -> str:
        """Formats the state's name, as `str.__format__` does.

        Args:
            format_spec: The format specification, such as `">10"`.

        Returns:
            The formatted name.
        """

class TesTask:
    """A TES task: one to create, or one as returned by the server.

    Keeps the task's full JSON, including fields that the TES models don't know. Missing
    fields are `None`. Nested values, such as executors, come as `dict`s and `list`s.
    Tasks can be pickled and copied.
    """

    def __init__(self, task: TesTask | Mapping[str, Any] | str) -> None:
        """Builds a task and checks it against the TES models.

        Args:
            task: A `TesTask`, a `dict` (or other JSON-serialisable mapping), or JSON text.
                Unlike for `TES.create()`, a `str` is JSON, not a path.

        Raises:
            ValueError: The task doesn't fit the TES models.
        """
    @classmethod
    def from_dict(cls, task: Mapping[str, Any]) -> Self:
        """Builds a task from a `dict`, as `cls(task)`.

        Args:
            task: The task's JSON as a mapping.

        Returns:
            The task.

        Raises:
            ValueError: The task doesn't fit the TES models.
        """
    @classmethod
    def from_json(cls, text: str) -> Self:
        """Builds a task from JSON text.

        Args:
            text: The task as a JSON object.

        Returns:
            The task.

        Raises:
            ValueError: `text` isn't JSON, or the task doesn't fit the TES models.
        """
    @classmethod
    def from_file(cls, path: str | os.PathLike[str]) -> Self:
        """Reads a task from a JSON file.

        Args:
            path: The file's path.

        Returns:
            The task.

        Raises:
            OSError: The file can't be read, for example `FileNotFoundError`.
            ValueError: The file isn't JSON, or the task doesn't fit the TES models.
        """
    @property
    def id(self) -> str | None:
        """The task's ID; `None` for a task that isn't created yet."""
    @property
    def state(self) -> TesState:
        """The task's state; `UNKNOWN` if it has none or one that TES 1.1 doesn't define."""
    @property
    def status(self) -> TesState:
        """Same as `state`."""
    @property
    def name(self) -> str | None:
        """The task's name."""
    @property
    def description(self) -> str | None:
        """The task's description."""
    @property
    def creation_time(self) -> str | None:
        """When the task was created, as an RFC 3339 timestamp."""
    @property
    def inputs(self) -> list[_JSONObject] | None:
        """The files to put into the task's containers."""
    @property
    def outputs(self) -> list[_JSONObject] | None:
        """The files to keep from the task's containers."""
    @property
    def resources(self) -> _JSONObject | None:
        """The resources that the task asks for, such as CPU cores and RAM."""
    @property
    def executors(self) -> list[_JSONObject] | None:
        """The commands to run, in order, each with its container image."""
    @property
    def volumes(self) -> list[str] | None:
        """Paths that the executors share."""
    @property
    def tags(self) -> dict[str, str] | None:
        """Key-value tags of the task."""
    @property
    def logs(self) -> list[_JSONObject] | None:
        """The task's logs, one per attempt (in the `BASIC` and `FULL` views)."""
    def json(self) -> str:
        """Gives the task as JSON text, as given or received; keys may come in another order."""
    def to_dict(self) -> _JSONObject:
        """Gives the task as a new `dict`, as given or received."""

PyTesTask = TesTask
"""Previous name of `TesTask`."""

class Task:
    """A TES task on the server, addressed by its ID."""

    def __init__(self, id: str, transport: Transport) -> None:
        """Addresses an existing task; sends no request.

        Args:
            id: The task's ID.
            transport: A transport with the TES service's configuration, such as
                `tes.transport`.
        """
    @property
    def id(self) -> str:
        """The task's ID."""
    @property
    def transport(self) -> Transport:
        """A transport with the task's configuration, for further endpoints."""
    def status(self) -> TesState:
        """Gets the task's current state.

        Returns:
            The state.

        Raises:
            RuntimeError: The request failed or the response has no state.
        """
    def cancel(self) -> None:
        """Asks the server to cancel the task.

        Raises:
            RuntimeError: The request failed.
        """
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: None = None,
    ) -> TesState: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str,
    ) -> TesTask: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str | None = None,
    ) -> TesState | TesTask:
        """Waits until the task ends; gives its final state, or the task in `view`.

        A task ends in `COMPLETE`, `EXECUTOR_ERROR`, `SYSTEM_ERROR`, `CANCELED` or
        `PREEMPTED`. Polls ask for the minimal view and start `poll_interval` seconds
        apart; the interval doubles after each poll. The wait releases the GIL; in the main
        thread, Ctrl-C stops it with `KeyboardInterrupt`. Nothing here cancels the task:
        call `cancel()` for that.

        Args:
            timeout: The most seconds for the whole wait; `None` waits as long as it takes.
            poll_interval: The seconds between the first polls.
            max_interval: The longest interval between polls; by default 10 seconds, or
                `poll_interval` if that's longer.
            view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case, to get the ended task in
                that view instead of its state.

        Returns:
            The final state, or the ended task in `view`.

        Raises:
            WaitTimeoutError: The time ran out; the task goes on running.
            RuntimeError: A poll failed; polls are not retried.
            ValueError: An argument is invalid, such as a negative `timeout`, a
                `poll_interval` of 0 or a `max_interval` below `poll_interval`.

        Example:
            ```python
            try:
                state = task.wait(timeout=3600)
            except WaitTimeoutError as error:
                print(error.id, "still", error.last_state)
            ```
        """

class WaitTimeoutError(TimeoutError):
    """Raised when `Task.wait()` or `AsyncTask.wait()` runs out of time.

    The task goes on running.
    """

    id: str | None
    """The task's ID; `None` on an error raised by other code."""
    last_state: TesState | None
    """The task's last polled state, or `None` if no poll succeeded."""

class ListTasksParams:
    """Filters and paging for `TES.list()`."""

    def __init__(
        self,
        name_prefix: str | None = None,
        state: TesState | str | None = None,
        tag_key: list[str] | None = None,
        tag_value: list[str] | None = None,
        page_size: int | None = None,
        page_token: str | None = None,
        view: str | None = None,
    ) -> None:
        """Builds the filters; unset ones are not sent.

        Args:
            name_prefix: The start of the tasks' names.
            state: A `TesState`, or a TES state name in any case.
            tag_key: Tag keys that the tasks have.
            tag_value: Values of the tags in `tag_key`, paired by position.
            page_size: The most tasks on the page.
            page_token: The `next_page_token` of the previous page.
            view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case.

        Raises:
            ValueError: `state` or `view` is not a TES state or view.
        """
    @property
    def name_prefix(self) -> str | None:
        """The start of the tasks' names."""
    @property
    def state(self) -> TesState | None:
        """The state filter."""
    @property
    def tag_key(self) -> list[str] | None:
        """Tag keys that the tasks have."""
    @property
    def tag_value(self) -> list[str] | None:
        """Values of the tags in `tag_key`, paired by position."""
    @property
    def page_size(self) -> int | None:
        """The most tasks on the page."""
    @property
    def page_token(self) -> str | None:
        """The token of the page to get."""
    @property
    def view(self) -> str | None:
        """The view of the tasks, in uppercase."""

class ListTasksResponse:
    """One page of tasks from `TES.list()`."""

    @property
    def tasks(self) -> list[TesTask]:
        """The tasks of the page."""
    @property
    def next_page_token(self) -> str | None:
        """The token of the next page, for `ListTasksParams(page_token=...)`."""
    def json(self) -> str:
        """Gives the response as JSON text, as received."""
    def to_dict(self) -> _JSONObject:
        """Gives the response as a new `dict`, as received."""

class TES:
    """A TES client. Subclass it to add endpoints, using `self.transport`."""

    def __init__(self, config: Configuration) -> None:
        """Builds a client and checks the service.

        Args:
            config: The service's configuration.

        Raises:
            RuntimeError: The service isn't reachable or isn't a TES.
        """
    @property
    def config(self) -> Configuration:
        """The client's configuration."""
    @property
    def transport(self) -> Transport:
        """A transport with the client's configuration, for further endpoints."""
    def create(self, task: TesTask | Mapping[str, Any] | str | os.PathLike[str]) -> Task:
        """Creates a task.

        The task's JSON is sent as given, including fields that the TES models don't know.
        If creation fails after submission, the server may have created the task; don't
        retry automatically.

        Args:
            task: A `TesTask`, a `dict` (or other mapping), or the path of a JSON file as
                `str` or `os.PathLike`. A `str` is always a path; pass JSON text as
                `TesTask.from_json(text)`.

        Returns:
            The created task.

        Raises:
            ValueError: The task is invalid, or a `str` starts with `{`.
            OSError: The task file can't be read, for example `FileNotFoundError`.
            RuntimeError: The request failed or the response doesn't fit the TES models.
        """
    def get(self, view: str, id: str) -> TesTask:
        """Gets a task.

        Args:
            view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case.
            id: The task's ID.

        Returns:
            The task in that view.

        Raises:
            ValueError: `view` is not a TES view.
            RuntimeError: The request failed or the response doesn't fit the TES models.
        """
    def list(self, params: ListTasksParams | None = None) -> ListTasksResponse:
        """Lists tasks, one page at a time.

        Args:
            params: Filters and paging; by default, the first page of all tasks.

        Returns:
            The page.

        Raises:
            RuntimeError: The request failed or the response doesn't fit the TES models.
        """

class AsyncTES:
    """A TES client for `asyncio`: `create`, `get` and `list` return awaitables.

    The methods return `asyncio.Future`s of the running event loop, so they must be called
    from it. `AsyncTES(config)` doesn't contact the server; `await AsyncTES.connect(config)`
    first checks that `config` points at a TES service; otherwise the first call does.
    Invalid arguments raise when a method is called; failed requests raise `RuntimeError`
    when awaited. Cancelling an awaitable drops its request.
    """

    def __init__(self, config: Configuration) -> None:
        """Builds a client; sends no request.

        Args:
            config: The service's configuration.
        """
    @classmethod
    def connect(cls, config: Configuration, *args: Any, **kwargs: Any) -> asyncio.Future[Self]:
        """Builds `cls(config, *args, **kwargs)`; gives it once it has checked the service.

        Args:
            config: The service's configuration.
            *args: Further arguments of a subclass's constructor.
            **kwargs: Further keyword arguments of a subclass's constructor.

        Returns:
            A future of the client; it raises `RuntimeError` if the service isn't
            reachable or isn't a TES.
        """
    @property
    def config(self) -> Configuration:
        """The client's configuration."""
    @property
    def transport(self) -> Transport:
        """A transport with the client's configuration; its methods are synchronous."""
    def create(
        self, task: TesTask | Mapping[str, Any] | str | os.PathLike[str]
    ) -> asyncio.Future[AsyncTask]:
        """Creates a task; takes the same `task` as `TES.create()`.

        Args:
            task: A `TesTask`, a `dict` (or other mapping), or the path of a JSON file.

        Returns:
            A future of the created task.

        Raises:
            ValueError: The task is invalid.
            OSError: The task file can't be read.
        """
    def get(self, view: str, id: str) -> asyncio.Future[TesTask]:
        """Gets a task.

        Args:
            view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case.
            id: The task's ID.

        Returns:
            A future of the task in that view.

        Raises:
            ValueError: `view` is not a TES view.
        """
    def list(self, params: ListTasksParams | None = None) -> asyncio.Future[ListTasksResponse]:
        """Lists tasks, one page at a time.

        Args:
            params: Filters and paging; by default, the first page of all tasks.

        Returns:
            A future of the page.
        """

class AsyncTask:
    """A TES task on the server, addressed by its ID, for `asyncio`."""

    def __init__(self, id: str, transport: Transport) -> None:
        """Addresses an existing task; sends no request.

        Args:
            id: The task's ID.
            transport: A transport with the TES service's configuration.
        """
    @property
    def id(self) -> str:
        """The task's ID."""
    @property
    def transport(self) -> Transport:
        """A transport with the task's configuration, for further endpoints."""
    def status(self) -> asyncio.Future[TesState]:
        """Gets the task's current state.

        Returns:
            A future of the state; it raises `RuntimeError` if the request fails.
        """
    def cancel(self) -> asyncio.Future[None]:
        """Asks the server to cancel the task.

        Returns:
            A future of `None`; it raises `RuntimeError` if the request fails.
        """
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: None = None,
    ) -> asyncio.Future[TesState]: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str,
    ) -> asyncio.Future[TesTask]: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str | None = None,
    ) -> asyncio.Future[TesState] | asyncio.Future[TesTask]:
        """Waits until the task ends, like `Task.wait()`.

        Cancelling the awaiting coroutine stops polling; it doesn't cancel the task.
        Between polls, the wait holds no call thread, so sleeping waits do not delay other
        async calls.

        Args:
            timeout: The most seconds for the whole wait; `None` waits as long as it takes.
            poll_interval: The seconds between the first polls.
            max_interval: The longest interval between polls; by default 10 seconds, or
                `poll_interval` if that's longer.
            view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, to get the ended task in that view.

        Returns:
            A future of the final state, or of the ended task in `view`. It raises
            `WaitTimeoutError` when the time runs out and `RuntimeError` if a poll fails.

        Raises:
            ValueError: An argument is invalid, when the method is called.
        """
