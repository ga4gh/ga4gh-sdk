# TES site check on Kubernetes

A one-off Kubernetes Job that checks a TES deployment, such as TESK, from
inside the cluster with the [`ga4gh-cli` image](../../docs/container-image.md).
It needs no public ingress to the TES API.

| File | Content |
| --- | --- |
| [`tes-site-check-job.yaml`](tes-site-check-job.yaml) | The Job |
| [`tes-site-check.sh`](tes-site-check.sh) | The check that the Job runs in the image |
| [`task.json`](task.json) | The disposable TES task: one `echo` in a pinned Alpine image, no inputs or outputs |

The check:

1. Submits the task once with `ga4gh-cli tes create` and prints
   `task_id=<id>` to stdout. `tes create` first reads the service's
   `service-info`, so an unreachable or misconfigured TES fails here.
2. Polls `tes status` every `POLL_SECONDS` (default 5) and prints each state
   to stderr, until the task ends or `TIMEOUT_SECONDS` (default 600) have
   passed. It then cancels the task.
3. Prints the task's `FULL` view (JSON) and `result=<state>` to stdout, for
   example `result=COMPLETE` or `result=EXECUTOR_ERROR`; `result=TIMEOUT` if
   it canceled the task.

The Job succeeds (exit status 0) only if the task is `COMPLETE`. Any failed
request, any other final state or the time limit makes it fail with exit
status 1.

The Job never retries: `backoffLimit: 0` and `restartPolicy: Never`. TES task
creation is not idempotent, so a retried Pod would submit a second task. If a
check fails, read its log, then start a new Job deliberately.

## Running the check

You need `kubectl` access to a namespace whose Pods can reach the TES API.
Replace the placeholders in the commands below:

- the namespace, `NAMESPACE`;
- the TES API root as seen from a Pod, `TES_BASE_PATH`, for example the
  cluster-internal Service of TESK,
  `http://tesk-api.tesk.svc.cluster.local:8080/ga4gh/tes/v1`;
- the image, `ghcr.io/ga4gh/ga4gh-cli:VERSION@sha256:DIGEST`, pinned by
  digest (see [Image](../../docs/container-image.md#image)).

```sh
namespace=NAMESPACE
tes_base_path=TES_BASE_PATH
image=ghcr.io/ga4gh/ga4gh-cli:VERSION@sha256:DIGEST

# The CLI configuration, as a Secret, because it may hold credentials
printf '{"TES": {"base_path": "%s"}}\n' "$tes_base_path" > config.json
kubectl --namespace "$namespace" create secret generic ga4gh-cli-config \
  --from-file=config.json
rm config.json

# The check script and the task
kubectl --namespace "$namespace" create configmap ga4gh-tes-site-check \
  --from-file=tes-site-check.sh --from-file=task.json

# The Job, with the pinned image
job=$(sed "s|ghcr.io/ga4gh/ga4gh-cli:VERSION@sha256:DIGEST|$image|" \
  tes-site-check-job.yaml \
  | kubectl --namespace "$namespace" create --filename - --output name)
kubectl --namespace "$namespace" wait --for=condition=complete --timeout=15m "$job"
kubectl --namespace "$namespace" logs "$job"
```

Run the commands from this directory. If `kubectl wait` times out or the Job
fails, `kubectl logs` still shows the task ID and the states. The Job and its
log are deleted a day after it ends (`ttlSecondsAfterFinished`).

For a TES that needs a token, add it to `config.json`, or set
`oauth_access_token_env` there and give the container that variable from a
Secret; see [Authentication](../../cli/README.md#authentication) for which
commands send it.

### Security settings

The Pod runs as UID 10001, without a service account token, with a read-only
root file system, no Linux capabilities and the `RuntimeDefault` seccomp
profile, so it is admitted under the `restricted` Pod Security Standard. On
platforms that assign their own UIDs, such as OpenShift, remove `runAsUser`,
`runAsGroup` and `fsGroup` from the manifest; the CLI still finds its
configuration, because `HOME` is set in the image.

### Clean up

```sh
kubectl --namespace "$namespace" delete secret ga4gh-cli-config
kubectl --namespace "$namespace" delete configmap ga4gh-tes-site-check
```

The test task stays in the TES's task list, tagged
`purpose: ga4gh-cli-site-check`.

## Testing the check

[`tests/run-image-tests.sh`](../../tests/run-image-tests.sh) runs
`tes-site-check.sh` in the image against a local Funnel, with the same
read-only mounts as the Job, for a task that completes, one that fails and
one that runs too long; CI runs it on every pull request.
