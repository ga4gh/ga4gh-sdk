# Usage guide

This guide covers the configuration file, the `ga4gh-cli` command-line tool
and the Rust library (`ga4gh-lib`, imported as `ga4gh_sdk`). For the Python
bindings, see the [Python README](../python/README.md). For installation, see
the [README](../README.md#installation).

## Available interfaces

| API | Rust library | CLI | Python bindings |
| --- | --- | --- | --- |
| Service Info | `ServiceInfo` | No direct command | `ServiceInfo` |
| TES | Tasks, listing, and polling | `tes create`, `list`, `get`, `status`, `cancel` | TES |
| DRS | Objects, access URLs, bulk, and `drs://` URIs | `drs get`, `access` | `DRS`, `DrsResolver` |
| WES | Runs and attachments | `wes run`, `list`, `get`, `status`, `cancel` | `WES`, `Run` |
| TRS | Tools and descriptors | `trs tools`, `tool`, `versions`, `version`, `descriptor`, `files`, `tests` | Not yet |
| Service Registry | `ServiceRegistry` | `registry list`, `get`, `types` | Not yet |

`ServiceInfo` reads the discovery endpoint of one configured service. The
separate GA4GH Service Registry lists services from a registry deployment.
The CLI reads one service section from the configuration file for each command.

## API root (`base_path`)

Every client takes a `Configuration` whose `base_path` is the **API root** of
one service, including the service's path prefix. The SDK resolves
`service-info`, `tasks`, `runs` and `objects` below that root:

| Service | Example `base_path`                  | Requests go to                        |
| ------- | ------------------------------------ | ------------------------------------- |
| TES     | `https://example.org/ga4gh/tes/v1`   | `…/ga4gh/tes/v1/service-info`, `…/tasks` |
| WES     | `https://example.org/ga4gh/wes/v1`   | `…/ga4gh/wes/v1/service-info`, `…/runs`  |
| DRS     | `https://example.org/ga4gh/drs/v1`   | `…/ga4gh/drs/v1/service-info`, `…/objects` |
| TRS     | `https://example.org/ga4gh/trs/v2`   | `…/ga4gh/trs/v2/service-info`, `…/tools` |
| Registry | `https://example.org/ga4gh/registry/v1` | `…/ga4gh/registry/v1/service-info`, `…/services` |

A service behind a gateway or reverse proxy keeps its full prefix, for example
`https://gateway.example.org/my-tes/ga4gh/tes/v1`. A trailing slash is
optional. The root must not contain a query string or a fragment. A host-only
URL such as `https://example.org` sends requests to `/service-info` on the
host, which usually fails with `404 Not Found`.

## Configuration file

The CLI, Rust `Configuration::from_file` and Python `Configuration.from_file()`
read `~/.ga4gh-cli/config.json`. The file holds one object per service, keyed by
the section name: `TES`, `WES`, `DRS`, `TRS`, or `REGISTRY` for the built-in
services, or any other name for a custom service:

```json
{
  "TES": {
    "base_path": "https://example.org/ga4gh/tes/v1",
    "basic_auth": {"username": "alice", "password": "change-me"},
    "oauth_access_token": "my-token",
    "timeouts": {"discovery": 30, "submission": 300}
  },
  "MY_SERVICE": {
    "base_path": "https://example.org/my-service/v1",
    "oauth_access_token_env": "MY_SERVICE_TOKEN",
    "queue": "long"
  }
}
```

- `base_path` (string, required): the API root described above.
- `basic_auth` (object, optional): `username` (string) and an optional
  `password` (string).
- `oauth_access_token` (string, optional): an access token.
- `oauth_access_token_env` (string, optional): the name of an environment
  variable that holds the access token, so that the token needn't be in the
  file. Use it in place of `oauth_access_token`, not together with it. If the
  variable isn't set, reading the section is an error.
- `timeouts` (object, optional): time limits in seconds, as positive numbers:
  `connect`, `request`, `discovery` and `submission`. Limits that the object
  doesn't name keep their defaults; see [Timeouts](#timeouts).
- Any other key: kept with its JSON value in `Configuration::extra` (Python:
  `Configuration.get_extra()`), for settings that the SDK doesn't use itself,
  such as those of a downstream crate. In the example, `extra` holds
  `queue`. Rust's `Debug` output shows only the keys, but `extra`
  and `get_extra()` themselves return the values as they are, so don't log
  them if they can hold credentials. The file doesn't set the other
  `Configuration` fields: `user_agent`, `bearer_access_token` and `api_key`
  in a section end up in `extra` as well.

A missing file or section, or a field of the wrong type, is an error. Rust
reads a custom section with `ServiceType::Custom("MY_SERVICE".into())`, Python
with `config.from_file("MY_SERVICE")`. `Configuration::from_path(path, "TES")`
reads a section from another file. Files written for earlier versions read as
before; their `extra` is empty.

The SDK reads `basic_auth` and `oauth_access_token` but **doesn't send them
yet**: requests carry no `Authorization` header (see
[#31](https://github.com/ga4gh/ga4gh-sdk/issues/31)). Until that changes, the
SDK and the CLI work only with services that allow anonymous access, or behind
a proxy that adds credentials.

The file can hold credentials, so keep it private, for example with
`chmod 600 ~/.ga4gh-cli/config.json`.

## Command-line tool

The CLI has TES commands (`create`, `list`, `get`, `status`, `cancel`), WES
commands (`run`, `list`, `get`, `status`, `cancel`), DRS commands (`get`,
`access`), TRS commands (`tools`, `tool`, `versions`, `version`, `descriptor`,
`files`, `tests`), and Service Registry commands (`list`, `get`, `types`). It
reads the matching `TES`, `WES`, `DRS`, `TRS`, or `REGISTRY` configuration
section, including its `timeouts`. The [CLI README](../cli/README.md) describes
each command, its output and exit codes. `ServiceInfo` is available from the
Rust library below.

## Timeouts

Every request has a time limit. `Configuration` holds them as `Timeouts`; the
Rust clients, the CLI and the Python bindings read them from there:

| Limit | Default | Applies to |
| --- | --- | --- |
| `connect` | 10 s | Opening each connection, including TLS, within any of the limits below. |
| `request` | 60 s | Each request that none of the limits below applies to, from sending it to reading the whole response. |
| `discovery` | 10 s | The `service-info` request with which the TES, WES, DRS and TRS clients check a service: `new`, `with_transport`, `discover`, `discover_with_transport` and `refresh`, and `TRS::service_info`. |
| `submission` | 60 s | One TES task or WES run submission: `TES::create`, `TES::create_value`, `WES::run_workflow`, `WES::run_workflow_with_attachments`, and the CLI and Python calls built on them, including uploads. The CLI's `wes run` allows 300 s unless the section sets a `submission` other than 60. |

`discovery` and `submission` replace `request` for their requests, so they can
be longer or shorter than `request`. Calls that take an `Operation`, such as
`WES::run_workflow_streaming` and `Transport::send_once`, use its deadline in
the same way. Waiting (`WaitOptions`), paging (`PagingOptions`) and DRS
resolution have a budget of their own for the whole call; each of their
requests also has the `request` limit, and whichever ends first stops it.
Other settings are not time limits of requests and don't depend on these: the
`max_age` of capability snapshots says how long cached service info counts as
current, and the `Retry-After` delays of retries stay within the call's budget.

A submission is sent once and never repeated. If it runs out of time, or loses
its connection, after it was sent, it fails with `AmbiguousSubmissionError`
(Python: `RuntimeError` with "submission may have been accepted"): the service
may have created the task or run, so look for it, for example by its tags,
before you submit it again. A submission that runs out of time before it was
sent fails with `OperationError` and sent nothing.

Set the limits in Rust with `Configuration::with_timeouts`, in a configuration
file section with `timeouts` (see [above](#configuration-file)), and in Python
with `Configuration.set_timeouts()`:

```rust
use ga4gh_sdk::utils::timeouts::Timeouts;
use std::time::Duration;

let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?)
    .with_timeouts(Timeouts {
        discovery: Duration::from_secs(30),
        submission: Duration::from_secs(300),
        ..Default::default()
    });
let tes = TES::new(&config).await?;
```

A client reads the limits when it is built: change them in the
`Configuration` before you build the client or `Transport`.
`Transport::with_timeouts(&config, connect, request)` builds a transport with
other `connect` and `request` limits. For other HTTP settings, such as TLS
roots, a proxy or default headers, build a `reqwest::Client`, wrap it with
`Transport::with_client(&config, client)` and pass the transport to
`TES::with_transport`, `WES::with_transport`, `DRS::with_transport` or
`TRS::with_transport` (and `TES::discover_with_transport`,
`WES::discover_with_transport`). The client's own connect and request timeouts
then replace `connect` and `request`; `discovery` and `submission` still come
from the transport's configuration.

## Rust library

Add this repository's library as a Git dependency. The package is `ga4gh-lib`;
its crate name is `ga4gh_sdk`. The clients are
`async`, so you also need an async runtime such as Tokio:

```toml
[dependencies]
ga4gh-lib = { git = "https://github.com/ga4gh/ga4gh-sdk" }
tokio = { version = "1", features = ["full"] }
url = "2"
serde_json = "1"
```

TES, WES, DRS, and TRS clients check the service's `service-info` when created
and return an error if the endpoint isn't reachable or has the wrong type. The
check and all other requests have time limits; see [Timeouts](#timeouts).

### Configuration

```rust
use ga4gh_sdk::utils::configuration::{BasicAuth, Configuration};
use url::Url;

// No authentication
let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?);

// Credentials are stored, but not yet sent with requests (see #31)
let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?)
    .with_oauth_access_token("my-token".to_owned());

// The token from an environment variable of your choice
let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?)
    .with_oauth_access_token_from_env("MY_SERVICE_TOKEN")?;

let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?)
    .with_basic_auth(BasicAuth {
        username: "alice".to_owned(),
        password: Some("change-me".to_owned()),
    });

// From ~/.ga4gh-cli/config.json
use ga4gh_sdk::clients::ServiceType;
let config = Configuration::from_file(ServiceType::TES)?;

// A custom section and its other keys
let config = Configuration::from_file(ServiceType::Custom("MY_SERVICE".into()))?;
let queue = config.extra.get("queue").and_then(|value| value.as_str());
```

`ServiceType` is `#[non_exhaustive]`, so a `match` on it needs a wildcard arm.

### ServiceInfo

```rust
use ga4gh_sdk::clients::serviceinfo::ServiceInfo;

let info = ServiceInfo::new(&config)?.get().await?;
println!("{} ({} {})", info.name, info.r#type.artifact, info.r#type.version);
```

### Service Registry

Use the registry's API root to discover service URLs. The registry client does
not require a token for public reads; an individual deployment may require
authentication.

```rust
use ga4gh_sdk::clients::registry::ServiceRegistry;
use ga4gh_sdk::utils::configuration::Configuration;

let root = url::Url::parse("https://registry.example.org/ga4gh/registry/v1")?;
let registry = ServiceRegistry::new(&Configuration::new(root));
for service in registry.list_services().await? {
    println!("{} {}", service.id, service.url);
}
```

`list_service_types()` and `get_service(id)` read the remaining registry
endpoints. For the CLI equivalent, see [Service Registry commands](../cli/README.md#service-registry-commands).

### TES

```rust
use ga4gh_sdk::clients::tes::models::{ListTasksParams, TesTask};
use ga4gh_sdk::clients::tes::{TesView, TES};

let tes = TES::new(&config).await?;

// From the repository root, with a TES service running as in the quick start:
let task_json = std::fs::read_to_string("examples/tes-hello.json")?;
let task: TesTask = serde_json::from_str(&task_json)?;
let task = tes.create(task).await?;
println!("created {}", task.id);
println!("state {:?}", task.status().await?);

let params = ListTasksParams {
    name_prefix: None,
    state: None,
    tag_key: None,
    tag_value: None,
    page_size: Some(10),
    page_token: None,
    view: Some(TesView::Minimal),
};
let tasks = tes.list_tasks(Some(params)).await?;

let details = tes.get(TesView::Full, &task.id).await?;
```

To cancel a task that is still running, call `task.cancel().await?`.

### WES

```rust
use ga4gh_sdk::clients::wes::models::RunRequest;
use ga4gh_sdk::clients::wes::WES;

let config = Configuration::new(Url::parse("https://example.org/ga4gh/wes/v1")?);
let wes = WES::new(&config).await?;

let mut request = RunRequest::new(
    "CWL".to_owned(),
    "v1.2".to_owned(),
    "https://example.org/workflows/hello.cwl".to_owned(),
);
request.workflow_params = Some(serde_json::json!({}));
let run = wes.run_workflow(request).await?;

println!("state {:?}", run.status().await?);
let runs = wes.list_runs(None, Some(10)).await?; // page token, page size
let log = wes.get_run(&run.id).await?;
run.cancel().await?;
```

`WES::run_workflow_with_attachments` uploads workflow files along with the
request, for services that accept `workflow_attachment`. Use a workflow URL and
type/version supported by the target WES service.

### DRS

```rust
use ga4gh_sdk::clients::drs::DRS;

let config = Configuration::new(Url::parse("https://example.org/ga4gh/drs/v1")?);
let drs = DRS::new(&config).await?;

let object = drs.get_object("my-object-id").await?;
println!("{} ({} bytes)", object.id, object.size);
let access = drs.get_access_url("my-object-id", "my-access-id").await?;
// Use access.url and any access headers to fetch the bytes; do not log signed URLs.
```

`get_object_once` and `get_access_url_once` return `Resolution::Pending` while the
server prepares an answer (`202 Accepted`); `resolve_object` and `resolve_access_url`
wait for it within an `Operation` budget. `get_bulk_objects` and
`get_bulk_access_urls` use the DRS 1.4 bulk endpoints and report an outcome per ID
under a `BulkPolicy`. `uri::DrsResolver` plans access for hostname-based `drs://`
URIs. The `_raw` variants (`get_object_raw`, `get_access_url_raw`,
`resolve_reference_raw`, `get_bulk_objects_raw`, `get_bulk_access_urls_raw`) keep the
server's JSON, including fields that the models don't know.

### TRS

```rust
use ga4gh_sdk::clients::trs::{ToolFilter, TRS};

let config = Configuration::new(Url::parse("https://workflowhub.eu/ga4gh/trs/v2")?);
let trs = TRS::new(&config).await?;
let tools = trs.list_tools(&ToolFilter {
    tool_class: Some("Workflow".to_owned()),
    limit: Some(10),
    ..Default::default()
}).await?;
for tool in tools {
    println!("{}", tool.id);
}
```

### Models

The request and response models (`clients::<api>::models`) are generated from
the specs. Servers often send more than the spec defines; the models keep it:

- Every model struct has a field `extra` (a `serde_json::Map`) with the
  fields that its spec doesn't define, at every level. Writing a model back,
  for example a task that was read, sends them again. To add a field of your
  own, insert it into `extra`. Don't use names of fields that the model
  defines: the JSON would then have the key twice.
- Enums keep values that the spec doesn't define in their `Unrecognized`
  variant, for example `DescriptorType::Unrecognized("JUPYTER")`, and write
  them back unchanged. `Display` gives the value as received. TRS clients
  send an unrecognized descriptor type as one encoded path segment and reject
  empty values, `.` and `..`.
- TES and WES task and run states are the exception: a state that the spec
  doesn't define reads as `TesState::Unknown` or `State::Unknown`, which both
  specs define as the safe default, and is written back as `UNKNOWN`. The
  server's value is lost; the Python `TesTask.to_dict()` still has it. The TES
  CLI and the Python bindings still reject unknown state names as list
  filters.

```rust
use ga4gh_sdk::clients::tes::models::{TesExecutor, TesTask};

let mut task = TesTask::new(vec![TesExecutor::new(
    "alpine".to_owned(),
    vec!["echo".to_owned(), "hello".to_owned()],
)]);
task.extra.insert("x-queue".to_owned(), serde_json::json!("long"));
```

Build models with their `new` function or with `..Default::default()`, so that
fields added later, such as `extra`, don't break your code. For WES,
`RunRequest.extra` fields are sent as further parts of the `multipart/form-data`
request: strings as they are, other values as JSON. Names of parts that the
request already has are an error.

### Errors

Clients return `Box<dyn std::error::Error>`. Non-2xx HTTP responses come back
as `ga4gh_sdk::utils::transport::HttpError` (formerly `HttpStatusError`, still
available under that name), which carries the status code and body. Downcast to
it to distinguish, for example, a `404` from a network error.
