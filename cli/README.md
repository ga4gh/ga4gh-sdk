# GA4GH-CLI

The `ga4gh-cli` is a command line tool that leverages `ga4gh-sdk` library to provide user-frienly interface to work with the GA4GH API services. Currently, it supports GA4GH ServiceInfo and TES APIs with HTTP Basic Authorization or Token-Based Authorization.

## Configuration

The `ga4gh-cli` expects configuration in `~/.ga4gh-cli/config.json` file. 

### Configuration Example

```json
{
    "TES": {
        "base_path": "http://localhost:8000",
        "basic_auth": {
            "username": "your_username",
            "password": "your_password"
        },
        "oauth_access_token": "your_oauth_access_token"
    }
}
```

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
