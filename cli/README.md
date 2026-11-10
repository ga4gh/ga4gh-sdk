# GA4GH-CLI

The `ga4gh-cli` uses the GA4GH SDK to work with TES and DRS services.

## Configuration

The `ga4gh-cli` expects configuration in `~/.ga4gh-cli/config.json` file. 

The file and the section for the selected service (`TES` or `DRS`) must exist;
the CLI reports an error instead of silently using `http://localhost` when
either is missing. Only `base_path` is required. `basic_auth` may omit
`password`, in which case no password is set.
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
    },
    "DRS": {
        "base_path": "https://drs.example.org/ga4gh/drs/v1"
    }
}
```

Each `base_path` is the service's API root, with or without a trailing slash.
The CLI requests `service-info` and other endpoints below it. Do not include
a query string or fragment. Configs that previously contained only the host
must add the deployed service prefix, such as `/ga4gh/tes/v1` or
`/ga4gh/drs/v1` in the example above.

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

1. To create a new task, save the task definition as JSON in a file:

```json
{
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
}
```

Then pass the file to the CLI:

```sh
ga4gh-cli tes create ./tests/sample.tes
```

You can also pass the task JSON directly or read it from standard input:

```sh
ga4gh-cli tes create '{"executors":[{"image":"alpine","command":["true"]}]}'
cat task.json | ga4gh-cli tes create -
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
The command prints the server's JSON unchanged, including fields unknown to
the current SDK models.

4. To retrieve the task status run the `tes status` command:

```sh
ga4gh-cli tes status [TASK-ID]      
```

The command prints the TES state, such as `QUEUED`.

5. To cancel the task run the `tes cancel` command:

```sh
ga4gh-cli tes cancel [TASK-ID]      
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

## Extending the CLI in Rust

The `ga4gh-cli` package also exposes a Rust library. A downstream binary can
add its own Clap 4 subcommands to `ga4gh_cli::command()`. Call
`ga4gh_cli::dispatch()` for the built-in commands, or
`ga4gh_cli::dispatch_with_config()` to supply a service configuration directly.
Both return `None` when the downstream binary should handle its own command.
Built-in commands return stdout, stderr, and an exit code in `CommandOutput`,
so the downstream binary can write or reformat their output. See the
[downstream CLI test](tests/downstream_cli.rs) for a working example.
