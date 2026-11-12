# Container image

The `ga4gh-cli` OCI image runs the command-line tool as a short-lived
container, for example as a Kubernetes Job next to a TES deployment. The
container can then reach services and storage that are only available inside
a cluster or site network, without installing tools on its nodes. It is a
client image: it is not a TES server and not an image for TES task
executors.

## Image

| | |
| --- | --- |
| Name | `ghcr.io/ga4gh/ga4gh-cli` |
| Tags | The workspace version without the `v` of the release tag, for example `0.2.0` or `0.2.0-rc.1`. There is no `latest` tag. |
| Platforms | `linux/amd64`, `linux/arm64` |
| Entrypoint | `ga4gh-cli`; the default command is `--help` |
| User | `ga4gh`, UID and GID 10001 |
| `HOME` | `/home/ga4gh` |
| Working directory | `/work` |
| Base | Debian 13 (`trixie-slim`), with OpenSSL 3 and the Debian CA certificates |
| Provenance | A [GitHub build provenance attestation](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations) for each published image |

The [Release workflow](../.github/workflows/release.yml) publishes the image
for each version tag (see the [release guide](releasing.md#cli-image)).
Until the first release that includes it, build the image yourself (see
[Building the image](#building-the-image)).

Use a version and pin its digest, so that every run uses exactly the same
image:

```sh
docker buildx imagetools inspect ghcr.io/ga4gh/ga4gh-cli:0.2.0
# Name: ghcr.io/ga4gh/ga4gh-cli:0.2.0, Digest: sha256:...
image=ghcr.io/ga4gh/ga4gh-cli:0.2.0@sha256:...
gh attestation verify "oci://$image" --repo ga4gh/ga4gh-sdk
```

## Running the CLI

The image contains no configuration, credentials or task data. Supply them
at run time:

- **Configuration:** the CLI reads `$HOME/.ga4gh-cli/config.json`, that is
  `/home/ga4gh/.ga4gh-cli/config.json` (see
  [Configuration](../cli/README.md#configuration)). Mount the file or its
  directory read-only. The file must be readable by UID 10001, or by the UID
  that you run the container with. `HOME` stays `/home/ga4gh` for any UID.
- **Credentials:** keep them out of the image and out of task files. Put a
  token in the config file that you mount, or name an environment variable
  with `oauth_access_token_env` and pass that variable to the container
  (`--env`, or a Kubernetes Secret). The CLI doesn't send credentials for
  most commands yet (see [Authentication](../cli/README.md#authentication)).
- **Task files and outputs:** mount them under `/work`, the working
  directory, so that relative paths work. Mount output directories writable
  for the container's UID.

```sh
docker run --rm \
  --volume "$HOME/.ga4gh-cli:/home/ga4gh/.ga4gh-cli:ro" \
  --volume "$PWD/examples:/work:ro" \
  "$image" tes create tes-hello.json
```

The container's exit status is the CLI's: 0 on success, 1 for request,
server and input errors, 2 for usage errors. Results go to stdout, errors and
logs to stderr. To reach a TES on the host, such as a local Funnel, add
`--network host` on Linux. Set time limits for slow or unreachable services
with `timeouts` in the configuration file (see
[Configuration](../cli/README.md#configuration)).

The image works with a read-only root file system and without Linux
capabilities (`--read-only --cap-drop ALL --security-opt no-new-privileges`),
as long as outputs go to a mounted volume.

## TES site check on Kubernetes

[`examples/kubernetes/`](../examples/kubernetes/README.md) has a Kubernetes
Job that checks a TES deployment, such as TESK, from inside the cluster: it
submits one small, disposable task, prints its ID, waits until it ends and
prints its `FULL` view. The Job exits 0 only if the task completes, and 1
otherwise; if it stops waiting before the task ends, it cancels the task. It
needs no public ingress to the TES API.

The Job runs once (`backoffLimit: 0`, `restartPolicy: Never`), because TES
task creation is not idempotent: a retried Pod would submit a second task.
Don't use the image as a general task submitter in Jobs or workflow steps
that Kubernetes or an engine may retry, unless the caller handles duplicate
submissions.

## Site-level test against a real TES

CI checks the image against a local Funnel only (see
[Testing the image](#testing-the-image)); it never contacts a real TES. To
test the image against your own TES, an operator can run the site check on
demand:

1. Follow the steps in [`examples/kubernetes/`](../examples/kubernetes/README.md)
   with the TES `base_path` that the cluster's Pods use, for example the
   cluster-internal Service of TESK.
2. Check that the Job succeeded and keep its log, which has the task ID and
   the final state:

   ```sh
   kubectl --namespace "$namespace" wait --for=condition=complete --timeout=15m "$job"
   kubectl --namespace "$namespace" logs "$job"
   ```

3. Delete the test task from the TES if the service keeps tasks and your
   site requires it; it is tagged `purpose: ga4gh-cli-site-check`.

Outside Kubernetes, run the same script with Docker on a host that can reach
the TES:

```sh
docker run --rm \
  --volume "$HOME/.ga4gh-cli:/home/ga4gh/.ga4gh-cli:ro" \
  --volume "$PWD/examples/kubernetes:/etc/ga4gh-site-check:ro" \
  --env TASK_FILE=/etc/ga4gh-site-check/task.json \
  --entrypoint sh "$image" /etc/ga4gh-site-check/tes-site-check.sh
```

## Building the image

From the repository root:

```sh
docker build --tag ga4gh-cli .
docker run --rm ga4gh-cli --version
```

The [`Dockerfile`](../Dockerfile) builds the CLI from the checked-out
sources with the workspace lockfile (`cargo build --locked`) in a pinned
Rust image, then copies only the `ga4gh-cli` binary into a pinned Debian
image. Both base images are pinned by digest; the Debian packages (OpenSSL,
CA certificates, build headers) come from the Debian archive at build time.
The build uses the Rust channel of
[`rust-toolchain.toml`](../rust-toolchain.toml), without its `clippy` and
`rustfmt` components. When you change the channel, change the Rust image in
the `Dockerfile` to the same version; `tests/run-image-tests.sh` fails while
they differ. Dependabot updates the digests of both base images, but not the
Rust version.

[`.dockerignore`](../.dockerignore) admits only the workspace manifests, the
lockfile, `README.md`, `LICENSE` and the member crates `lib/`, `cli/` and
`python/`. Within them, it leaves out environment files (except
`.env.example`), keys and certificates, credential files and stores (such as
`.ssh/`, `.kube/`, `.aws/`, `.cargo/`, `.netrc`), `target/` and logs. Only the
`ga4gh-cli` binary goes from the build stage into the image.

## Testing the image

```sh
tests/run-image-tests.sh
```

The script builds the image and checks the build context, the entrypoint,
the version, the user and the absence of sources in the image. If `funnel` is
on `PATH`, it also runs the site check script in the image against a local
Funnel: for a task that completes, one that fails, one that runs too long,
an unreachable TES and an invalid setting.
The [integration test guide](../tests/README.md#cli-image) has the details.
The **CLI image** job of CI runs it on `linux/amd64` (with Funnel) and
`linux/arm64` (without) for every pull request.
