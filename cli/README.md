# GA4GH-CLI

The `ga4gh-cli` uses the GA4GH SDK to work with GA4GH API services. It
currently provides TES, WES, DRS, TRS and Service Registry commands. The Rust
library also provides a `ServiceInfo` client (see the
[usage guide](../docs/usage.md)).

## Installation

From the repository root:

```sh
cargo install --path cli --locked
ga4gh-cli --help
```

## Configuration

The `ga4gh-cli` expects configuration in `~/.ga4gh-cli/config.json` file.

The file and the section for the selected service (`TES`, `WES`, `DRS`, `TRS`
or `REGISTRY`) must exist;
the CLI reports an error instead of silently using `http://localhost` when
either is missing. Only `base_path` is required. `basic_auth` may omit
`password`, in which case no password is set.
Present authentication fields must have the documented types; malformed values
are errors rather than being ignored.

### Configuration Example

```json
{
    "TES": {
        "base_path": "http://localhost:8000/ga4gh/tes/v1"
    },
    "WES": {
        "base_path": "https://wes.example.org/ga4gh/wes/v1"
    },
    "DRS": {
        "base_path": "https://drs.example.org/ga4gh/drs/v1"
    },
    "TRS": {
        "base_path": "https://workflowhub.eu/ga4gh/trs/v2"
    },
    "REGISTRY": {
        "base_path": "https://registry.example.org/ga4gh/registry/v1"
    }
}
```

Each `base_path` is the service's API root, with or without a trailing slash.
The CLI requests `service-info` and other endpoints below it. Do not include
a query string or fragment. Configs that previously contained only the host
must add the deployed service prefix, such as `/ga4gh/tes/v1`,
`/ga4gh/wes/v1` or `/ga4gh/drs/v1` in the example above. A service behind a
gateway keeps its whole prefix, for example
`https://gateway.example.org/my-tes/ga4gh/tes/v1`.

### Authentication

The CLI reads `basic_auth` and `oauth_access_token`, but doesn't send them
with requests yet (see
[#31](https://github.com/ga4gh/ga4gh-sdk/issues/31)). For now, use services
that allow anonymous access, or a proxy that adds credentials. Keep the file
private (`chmod 600 ~/.ga4gh-cli/config.json`), since it can hold
credentials. To keep a token out of the file, name an environment variable
that holds it with `oauth_access_token_env`; the
[usage guide](../docs/usage.md#configuration-file) describes all keys.

## Usage

### Basic local setup

For a local TES server, install Funnel and Docker, then:

1. Install Funnel following the [official documentation](https://ohsu-comp-bio.github.io/funnel/download/).

2. Run the Funnel server on port 8000:

```sh
funnel server run --Server.HTTPPort=8000
```

3. Configure the CLI in `~/.ga4gh-cli/config.json` using the example above.
Only `base_path` is needed. Docker must be running for Funnel to execute tasks.

### CLI Examples

1. From the repository root, submit the checked-in
[example task](../examples/tes-hello.json). It runs `echo` in an Alpine
container and has no external inputs or outputs. Save the returned ID for later
commands:

```sh
task_id=$(ga4gh-cli tes create examples/tes-hello.json)
printf '%s\n' "$task_id"
```

The CLI also accepts JSON text directly or from standard input:

```sh
ga4gh-cli tes create "$(cat examples/tes-hello.json)"
cat examples/tes-hello.json | ga4gh-cli tes create -
```

The command prints only the new task ID. If the service rejects it, the CLI
prints the HTTP status and server message to stderr and exits with code 1.

2. To retrieve the list of tasks run `tes list` command:

```sh
ga4gh-cli tes list
```

Repeat `-k` and `-v` to filter by several tags. Values match keys in the
same order. When the response includes a next page token, the CLI prints it
after the task table; pass it to `-t` to fetch the next page:

```sh
ga4gh-cli tes list -k project -v analysis -k owner -v alice
ga4gh-cli tes list -t 'next page token'
```

To fetch successive pages as one JSON object per line, use:

```sh
ga4gh-cli tes list --all --format jsonl
```

`--max-pages` and `--max-items` limit the export. If the limit stops listing
before the final page, the command exits with code 1 and writes a resume
checkpoint to stderr; already written JSONL stays on stdout.

Example output:

```sh
TASK ID                   State
csei52hrqek3h222k9f0      Complete
csei52hrqek3h222k9eg      Initializing
csei52hrqek3h222k9e0      Canceled
csei52hrqek3h222k9dg      Running
csei52hrqek3h222k9df      Queued
csei52hrqek3h222k9de      Queued
```

3. To get task details, run the `tes get` command:

```sh
ga4gh-cli tes get "$task_id" FULL
```

Possible `VIEW` values: `MINIMAL`, `BASIC` and `FULL`, in any case. `tes list`
takes the same values with `-w`.
The command prints the server's JSON unchanged, including fields unknown to
the current SDK models.

4. To retrieve the task status run the `tes status` command:

```sh
ga4gh-cli tes status "$task_id"
```

The command prints the TES state, such as `QUEUED`.

5. To cancel a task that is still running, use its ID with `tes cancel`:

```sh
ga4gh-cli tes cancel 'running-task-id'
```

All five TES commands exit with code 0 on success and code 1 on request or
server errors. Missing commands or arguments use Clap's exit code 2.

### DRS examples

Add a `DRS` section with the service's API root to the configuration file, as
shown above. Use `drs get` to read object metadata, then use an `access_id` from
its `access_methods` to request a URL for the object's bytes:

```sh
ga4gh-cli drs get 'object-id'
ga4gh-cli drs access 'object-id' 'access-id'
```

Both commands print the server's JSON unchanged. The `access` result may
contain headers needed to fetch the bytes. Service failures print an error to
stderr and exit with code 1; missing arguments use exit code 2.

### WES examples

Add a `WES` section with the service's API root to the configuration file, as
shown above. The commands follow the TES commands: submit a run, then track it
by its ID.

1. To submit a workflow, run `wes run` with the workflow URL, the workflow type
   and the version of the type. Parameters are optional (an empty object by
   default) and, as with `tes create`, can be a JSON file, inline JSON or `-`
   for standard input. The command prints only the new run ID:

```sh
run_id=$(ga4gh-cli wes run 'https://example.org/workflows/main.cwl' \
    --type CWL --type_version v1.2 --params params.json)
printf '%s\n' "$run_id"
```

Repeat `--attachment` (`-a`) to upload workflow files with the request. Each
file is uploaded under the relative path given, so run the command from the
directory that the paths are relative to; absolute paths and `..` are
rejected. Symbolic links are followed, so check the files of a repository
that you don't trust before you attach them. `WORKFLOW_URL` can then name an
attached file, if the service supports attachments:

```sh
ga4gh-cli wes run main.cwl --type CWL --type_version v1.2 \
    -a main.cwl -a tools/step.cwl --params '{"reads": "a.fastq"}'
```

`--tag KEY=VALUE` and `--engine_parameter KEY=VALUE` can be repeated;
`--engine` and `--engine_version` select a workflow engine if the service
offers several. Input errors, such as invalid JSON or a missing attachment, are
reported before the CLI contacts the service.

2. To list runs, run `wes list`:

```sh
ga4gh-cli wes list
```

Example output:

```sh
RUN ID                                STATE
3f2a0b6e-58c2-4e8a-9d1e-0c5b7a1d2e3f  RUNNING
9c1d4e2a-7b3f-4a6e-8d5c-1e2f3a4b5c6d  COMPLETE
Next page token: next
```

As with `tes list`, `-p` sets the page size, `-t` passes the next page token,
and `--all --format jsonl` writes every run as one JSON object per line, with
the same `--max-pages` and `--max-items` limits.

3. To get the details of a run, including its request, logs and outputs, run
   `wes get`. It prints the server's JSON unchanged:

```sh
ga4gh-cli wes get "$run_id"
```

4. To get the state of a run, such as `RUNNING` or `COMPLETE`, run
   `wes status`:

```sh
ga4gh-cli wes status "$run_id"
```

5. To cancel a run, run `wes cancel`. It prints the ID of the canceled run:

```sh
ga4gh-cli wes cancel "$run_id"
```

All five WES commands exit with code 0 on success and code 1 on request or
server errors; missing commands or arguments use exit code 2. In the table,
the printed run IDs and the error messages, control and bidirectional
formatting characters from the server are escaped, such as `\u{1b}`; JSONL
records use JSON escapes such as `\u001b`. Like `tes get` and `drs get`,
`wes get` prints the server's JSON as received.

### TRS examples

The `trs` commands browse a GA4GH Tool Registry Service (TRS), such as
WorkflowHub or Dockstore, and download workflow files from it. They only read
from the registry; nothing is changed there.

#### 1. Configure the registry

The `trs` commands need to know which registry to talk to. They read it from
the `TRS` section of the CLI's configuration file, `~/.ga4gh-cli/config.json`
(see [Configuration](#configuration)). The CLI uses one registry at a time.

**a. Choose a registry.** Its `base_path` is the registry's TRS API root.

| Registry    | `base_path`                               | Tool IDs look like                  |
|-------------|-------------------------------------------|-------------------------------------|
| WorkflowHub | `https://workflowhub.eu/ga4gh/trs/v2`     | `26`                                |
| Dockstore   | `https://dockstore.org/api/ga4gh/trs/v2`  | `#workflow/github.com/org/repo`     |

**b. Add the `TRS` section to the configuration file.** For a file containing
only TRS configuration, the full JSON is:

```json
{
    "TRS": {
        "base_path": "https://workflowhub.eu/ga4gh/trs/v2"
    }
}
```

**c. Check the connection.** This lists one tool from the registry:

```sh
ga4gh-cli trs tools --limit 1
```

#### 2. Find a tool, pick a version, download its files

A registry holds **tools**. Each tool has **versions**, and each version has
one or more **descriptors**: workflow files written in a language such as CWL
or WDL. Replace `TOOL_ID` and `VERSION_ID` below with values returned by the
registry:

```sh
# Find tools. All filters are optional.
ga4gh-cli trs tools --tool-class Workflow --descriptor-type CWL --limit 5

# Look at one tool and its versions.
ga4gh-cli trs tool 'TOOL_ID'
ga4gh-cli trs versions 'TOOL_ID'
ga4gh-cli trs version 'TOOL_ID' 'VERSION_ID'

# See which files the version has, then download the main workflow file.
ga4gh-cli trs files 'TOOL_ID' 'VERSION_ID' --type CWL
ga4gh-cli trs descriptor 'TOOL_ID' 'VERSION_ID' --type CWL --plain > main.cwl
```

#### Commands

| Command | Result |
| --- | --- |
| `trs tools [filters]` | A list of tools, optionally filtered. |
| `trs tool <TOOL_ID>` | One tool and its versions. |
| `trs versions <TOOL_ID>` | The versions of a tool. |
| `trs version <TOOL_ID> <VERSION_ID>` | One version and its descriptor types. |
| `trs files <TOOL_ID> <VERSION_ID> --type <TYPE>` | The paths and types of its files. |
| `trs descriptor <TOOL_ID> <VERSION_ID> --type <TYPE>` | Its primary descriptor; `--path <RELATIVE_PATH>` selects another file. |
| `trs tests <TOOL_ID> <VERSION_ID> --type <TYPE>` | Its test parameter files. |

Results are JSON by default. `trs descriptor --plain` prints the descriptor
content without a JSON wrapper. `trs tests --plain` prints one inline test file;
if the server returns several files or only a URL, omit `--plain` to inspect
their JSON instead. The commands exit with code 0 on success and 1 on request
or server errors; missing arguments use exit code 2. Error messages and JSON
output escape terminal control and bidirectional formatting characters.

## Service Registry commands

The `registry` commands read a [GA4GH Service Registry](https://github.com/ga4gh-discovery/ga4gh-service-registry),
such as a [cloud-registry](https://github.com/elixir-cloud-aai/cloud-registry)
deployment, to find the base URLs of TES, WES and other services. They use the
`REGISTRY` section of `~/.ga4gh-cli/config.json`, which needs only `base_path`,
the registry's API root:

```json
{
    "REGISTRY": {
        "base_path": "https://registry.example.org/ga4gh/registry/v1"
    }
}
```

Replace `registry.example.org` with your registry's host.
The CLI requests `services`, `services/{id}` and `services/types` below this
path. Public reads need no token; a deployment that requires authentication
needs an authenticating proxy until [#31](https://github.com/ga4gh/ga4gh-sdk/issues/31)
adds configured authentication.

1. To list the services, optionally only those of one type artifact (case
   ignored), run `registry list`:

```sh
ga4gh-cli registry list --type tes
```

Example output:

```sh
ID               TYPE                 URL
org.example.tes  org.ga4gh:tes:1.1.0  https://example.org/ga4gh/tes/v1
```

2. To get one service as JSON, run `registry get` with its ID:

```sh
ga4gh-cli registry get org.example.tes
```

3. To list the service types in the registry, one `group:artifact:version` per
   line, run `registry types`:

```sh
ga4gh-cli registry types
```

The specification has no server-side filter, so `--type` fetches all services
and filters them in the CLI.

`list` and `types` print only fields of the Service Registry specification;
`get` also prints the fields that a registry adds. Control and bidirectional
formatting characters from the registry, in values and in error messages, are
printed escaped, such as `\n` or `\u{1b}`; `get` uses JSON escapes such as `\u001b`,
so its output stays valid JSON. The commands exit with code 0 on
success and code 1 on request or server errors. A 404 from `list` or `types`
adds a hint to check `base_path`; from `get`, a 404 means that no service has
that ID.

## Extending the CLI in Rust

The `ga4gh-cli` package also exposes a Rust library. A downstream binary can
add its own Clap 4 subcommands to `ga4gh_cli::command()`. Call
`ga4gh_cli::dispatch()` for the built-in commands, or
`ga4gh_cli::dispatch_with_config()` to supply a service configuration directly.
Both return `None` when the downstream binary should handle its own command.
Built-in commands return stdout, stderr, and an exit code in `CommandOutput`,
so the downstream binary can write or reformat their output. See the
[downstream CLI test](tests/downstream_cli.rs) for a working example. A
downstream command can read its own section of the configuration file with
`Configuration::from_file(ServiceType::Custom("MY_SERVICE".into()))`; keys
that the SDK doesn't use itself are in `Configuration::extra`.
