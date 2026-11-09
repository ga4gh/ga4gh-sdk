# GA4GH-CLI

The `ga4gh-cli` is a command line tool that leverages `ga4gh-sdk` library to provide user-frienly interface to work with the GA4GH API services. Currently, it supports GA4GH ServiceInfo and TES APIs with HTTP Basic Authorization or Token-Based Authorization.

## Configuration

The `ga4gh-cli` expects configuration in `~/.ga4gh-cli/config.json` file. 

The file and its `TES` section must exist; the CLI reports an error instead of
silently using `http://localhost` when either is missing. Only `base_path` is
required. `basic_auth` may omit `password`, in which case no password is set.
Present authentication fields must have the documented types; malformed values
are errors rather than being ignored.

### Configuration Example

```json
{
    "TES": {
        "base_path": "http://localhost:8000/ga4gh/tes/v1",
        "basic_auth": {
            "username": "your_username",
            "password": "your_password"
        },
        "oauth_access_token": "your_oauth_access_token"
    }
}
```

`base_path` is the TES API root, with or without a trailing slash. The CLI
requests `service-info` and `tasks` below this path. Do not include a query
string or fragment in `base_path`.
Configs that previously contained only the host must add the deployed TES
service prefix, such as `/ga4gh/tes/v1` in the example above.

## Usage 

### Basic local setup

In order to quickly setup a TES server locally, you can use Funnel. Dockerized Funnel version doesn't have ServiceInfo end-point on the expected path, which is requirement by the GA4GH-SDK.

1. Deploy Funnel locally following the [offical documentation](https://ohsu-comp-bio.github.io/funnel/download/).

2. Run Funnel server with the port specifed:

```sh
funnel server start --Server.HTTPPort=[available-port]
```

3. Configure the CLI in `~/.ga4gh-cli/config.json` using example above. Only the `base_path` needed.

### CLI Examples

1. To create a new task run the `tes create` command. You can provide task definition json data as text:

```sh
ga4gh-cli tes create '{
    "name": "Hello world",
    "inputs": [{
        "url": "s3://funnel-bucket/hello.txt",
        "path": "/inputs/hello.txt"
    }],
    "outputs": [{
        "url": "s3://funnel-bucket/output.txt",
        "path": "/outputs/stdout"
    }],
    "executors": [{
        "image": "alpine",
        "command": ["cat", "/inputs/hello.txt"],
        "stdout": "/outputs/stdout"
    }]
}'
```

Or as a file:

```sh
ga4gh-cli tes create ./tests/sample.tes
```

The command prints the ID of the new task. If the service doesn't accept the task, it prints the error to stderr, and the exit code stays 0.

2. To retrieve the list of tasks run `tes list` command:

```sh
ga4gh-cli tes list
```

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

3. To get the infromation about the task run the `tes get` command:

```sh
ga4gh-cli tes get [TASK-ID] [VIEW]
```

Possible `VIEW` option values: `BASIC`, `FULL`.

4. To retrieve the task status run the `tes status` command:

```sh
ga4gh-cli tes status [TASK-ID]      
```

5. To cancel the task run the `tes cancel` command:

```sh
ga4gh-cli tes cancel [TASK-ID]      
```

## Extending the CLI in Rust

The `ga4gh-cli` package also exposes a Rust library. A downstream binary can
add its own Clap 4 subcommands to `ga4gh_cli::command()`. Call
`ga4gh_cli::dispatch()` for the built-in commands, or
`ga4gh_cli::dispatch_with_config()` to supply a TES configuration directly.
Both return `None` when the downstream binary should handle its own command.
Built-in commands return stdout, stderr, and an exit code in `CommandOutput`,
so the downstream binary can write or reformat their output. See the
[downstream CLI test](tests/downstream_cli.rs) for a working example.
