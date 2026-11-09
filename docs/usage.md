# Usage guide

This guide covers the configuration file, the `ga4gh-cli` command-line tool
and the Rust library (`ga4gh-lib`, imported as `ga4gh_sdk`). For the Python
bindings, see the [Python README](../python/README.md). For installation, see
the [README](../README.md#installation).

## API root (`base_path`)

Every client takes a `Configuration` whose `base_path` is the **API root** of
one service, including the service's path prefix. The SDK resolves
`service-info`, `tasks`, `runs` and `objects` below that root:

| Service | Example `base_path`                  | Requests go to                        |
| ------- | ------------------------------------ | ------------------------------------- |
| TES     | `https://example.org/ga4gh/tes/v1`   | `…/ga4gh/tes/v1/service-info`, `…/tasks` |
| WES     | `https://example.org/ga4gh/wes/v1`   | `…/ga4gh/wes/v1/service-info`, `…/runs`  |
| DRS     | `https://example.org/ga4gh/drs/v1`   | `…/ga4gh/drs/v1/service-info`, `…/objects` |

A service behind a gateway or reverse proxy keeps its full prefix, for example
`https://gateway.example.org/my-tes/ga4gh/tes/v1`. A trailing slash is
optional. The root must not contain a query string or a fragment. A host-only
URL such as `https://example.org` sends requests to `/service-info` on the
host, which usually fails with `404 Not Found`.

## Configuration file

The CLI, and `Configuration::from_file` in the library and the Python
bindings, read `~/.ga4gh-cli/config.json`. It holds one object per service
type, keyed by `TES`, `WES`, `DRS` or `TRS`:

```json
{
  "TES": {
    "base_path": "https://example.org/ga4gh/tes/v1",
    "basic_auth": null,
    "oauth_access_token": null
  }
}
```

- `base_path` (string, required): the API root described above.
- `basic_auth` (object or `null`): `{"username": "…", "password": "…"}`.
- `oauth_access_token` (string or `null`): an access token.

The SDK reads `basic_auth` and `oauth_access_token` but **doesn't send them
yet**: requests carry no `Authorization` header (see
[#31](https://github.com/ga4gh/ga4gh-sdk/issues/31)). Until that changes, the
SDK and the CLI work only with services that allow anonymous access, or behind
a proxy that adds credentials.

Write all three keys in each service object and set the unused ones to
`null`: the current loader stops with a panic (`no entry found for key`) if
`basic_auth` or `oauth_access_token` is missing.

If the file doesn't exist, the loader logs a warning and falls back to
`http://localhost` as the API root.

The file can hold credentials, so keep it private, for example with
`chmod 600 ~/.ga4gh-cli/config.json`.

## Command-line tool

The CLI currently has TES commands only. It reads the `TES` section of the
configuration file.

```console
$ ga4gh-cli --help
CLI to manage tasks

Usage: ga4gh-cli <COMMAND>

Commands:
  tes   TES subcommands
  help  Print this message or the help of the given subcommand(s)

$ ga4gh-cli tes --help
Commands:
  create  Create a task
  list    list all tasks
  get     get task data
  status  get status of the task
  cancel  cancel the task
```

Create a task from a TES task document (JSON) and print its ID:

```sh
ga4gh-cli tes create tests/sample.tes
```

List tasks, with optional filters and paging (`ga4gh-cli tes list --help`
shows all options):

```sh
ga4gh-cli tes list
ga4gh-cli tes list --name_prefix my- --page_size 10 --view MINIMAL
```

Show a task (`MINIMAL`, `BASIC` or `FULL` view), its state, or cancel it:

```sh
ga4gh-cli tes get <task-id> FULL
ga4gh-cli tes status <task-id>
ga4gh-cli tes cancel <task-id>
```

Set `RUST_LOG=debug` to see the requests the CLI makes. Some request failures
(for example in `create` and `list`) currently print an error but still exit
with status 0.

## Rust library

Add the library as a Git dependency (it isn't published on crates.io yet).
The package is `ga4gh-lib`; its crate name is `ga4gh_sdk`. The clients are
`async`, so you also need an async runtime such as Tokio:

```toml
[dependencies]
ga4gh-lib = { git = "https://github.com/ga4gh/ga4gh-sdk" }
tokio = { version = "1", features = ["full"] }
url = "2"
serde_json = "1"
```

Each client checks the service's `service-info` when it is created and returns
an error if the endpoint isn't reachable or isn't of the expected type.

### Configuration

```rust
use ga4gh_sdk::utils::configuration::{BasicAuth, Configuration};
use url::Url;

// No authentication
let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?);

// Credentials are stored, but not yet sent with requests (see #31)
let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?)
    .with_oauth_access_token(std::env::var("GA4GH_TOKEN")?);

let config = Configuration::new(Url::parse("https://example.org/ga4gh/tes/v1")?)
    .with_basic_auth(BasicAuth {
        username: "alice".to_owned(),
        password: Some("change-me".to_owned()),
    });

// From ~/.ga4gh-cli/config.json
use ga4gh_sdk::clients::ServiceType;
let config = Configuration::from_file(ServiceType::TES)?;
```

### Service info

```rust
use ga4gh_sdk::clients::serviceinfo::ServiceInfo;

let info = ServiceInfo::new(&config)?.get().await?;
println!("{} ({} {})", info.name, info.r#type.artifact, info.r#type.version);
```

### TES

```rust
use ga4gh_sdk::clients::tes::models::{ListTasksParams, TesTask};
use ga4gh_sdk::clients::tes::TES;

let tes = TES::new(&config).await?;

let task: TesTask = serde_json::from_str(&std::fs::read_to_string("tests/sample.tes")?)?;
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
    view: Some("MINIMAL".to_owned()),
};
let tasks = tes.list_tasks(Some(params)).await?;

let details = tes.get("FULL", &task.id).await?;
task.cancel().await?;
```

### WES

```rust
use ga4gh_sdk::clients::wes::models::WesRunRequest;
use ga4gh_sdk::clients::wes::WES;

let config = Configuration::new(Url::parse("https://example.org/ga4gh/wes/v1")?);
let wes = WES::new(&config).await?;

let mut request = WesRunRequest::new();
request.workflow_url = Some("https://github.com/nextflow-io/hello".to_owned());
request.workflow_type = Some("NFL".to_owned());
request.workflow_type_version = Some("DSL2".to_owned());
let run = wes.run_workflow(request).await?;

println!("state {:?}", run.status().await?);
let runs = wes.list_runs(None, Some(10)).await?; // page token, page size
let log = wes.get_run(&run.id).await?;
run.cancel().await?;
```

`WES::run_workflow_with_attachments` uploads workflow files along with the
request, for services that accept `workflow_attachment`.

### DRS

```rust
use ga4gh_sdk::clients::drs::DRS;

let config = Configuration::new(Url::parse("https://example.org/ga4gh/drs/v1")?);
let drs = DRS::new(&config).await?;

let object = drs.get_object("my-object-id").await?;
println!("{} ({} bytes)", object.id, object.size);
let access = drs.get_access_url("my-object-id", "my-access-id").await?;
println!("{}", access.url);
```

### Errors

Clients return `Box<dyn std::error::Error>`. Non-2xx HTTP responses come back
as `ga4gh_sdk::utils::transport::HttpStatusError`, which carries the status
code; downcast to it to tell, for example, a `404` from a network error.
