## Motivation

The models in `lib/src/clients/<api>/models/` are generated from the
[OpenAPI](https://www.openapis.org/) specification of each GA4GH API. They
hold the request and response bodies of the API calls. Regenerate them when
a spec version changes, or generate models for a new API.

The `models/` folders are generated code: every run replaces them. Don't edit
them by hand. Hand edits go into a patch (see [Hand edits](#hand-edits)), and
hand-written types go next to `models/`, for example
`lib/src/clients/tes/list_tasks_params.rs`. (`tes.patch` re-exports it from
`tes::models` so that the existing import path keeps working.)

## Pinned specs

`utils/build_models_wrapper.sh` lists the spec of each API, pinned to a tag:

| API | Spec | Patch |
|-----|------|-------|
| `serviceinfo` | [Service Info 1.0.0](https://github.com/ga4gh-discovery/ga4gh-service-info/tree/v1.0.0) | – |
| `tes` | [TES 1.1](https://github.com/ga4gh/task-execution-schemas/tree/v1.1) | `utils/patches/tes.patch` |
| `drs` | [DRS 1.4.0](https://github.com/ga4gh/data-repository-service-schemas/tree/drs-1.4.0) | `utils/patches/drs.patch` |
| `trs` | [TRS 2.0.1](https://github.com/ga4gh/tool-registry-service-schemas/tree/2.0.1) | – |
| `wes` | [WES 1.1.0](https://github.com/ga4gh/workflow-execution-service-schemas/tree/1.1.0) | `utils/patches/wes.patch` |

The models are generated with OpenAPI Generator 7.9.0 and formatted with
`rustfmt`. Running the wrapper on a clean checkout must not change any file in
`lib/src/clients`. If it does, a spec, the generator or a patch has changed.
No CI job checks this yet; run the wrapper and `git status lib/src/clients`
after changing a spec or a patch.

The WES models use the schema names of the spec (`RunLog`, `RunStatus`,
`State`, `TaskLog`, ...). Two of them are generated for `anyOf` lists and
cover both variants: `RunListResponseRunsInner` (`RunStatus` or `RunSummary`)
and `RunLogTaskLogsInner` (`Log` or `TaskLog`). The fields that only one
variant has are optional in them. `RunLog.request` reads as `None` if the echoed
request doesn't parse, for example because a required field is missing; the
rest of the run log stays readable. The WES `ServiceInfo` model
follows 1.1.0 and requires its new fields, so it doesn't parse the service
info of WES 1.0.1 servers; the client reads the generic Service Info model
instead.

## Prerequisites

- Bash, `patch`, and network access to `raw.githubusercontent.com` (the
  generator downloads the specs).
- One of:
  - [Docker](https://docs.docker.com/get-docker/): the script then runs the
    `openapitools/openapi-generator-cli:v7.9.0` image.
  - [OpenAPI Generator CLI](https://openapi-generator.tech/docs/installation/)
    as `openapi-generator-cli` on the `PATH`, with a Java runtime. The npm
    wrapper (`@openapitools/openapi-generator-cli`) is pinned to 7.9.0 by the
    script; other installations must be 7.9.0. The script checks the version.
- `rustfmt` (from `rustup component add rustfmt`). Without it, the models are
  left unformatted and the script prints a warning; run `cargo fmt --all`
  afterwards. Run the scripts from the repository root, so that `rustfmt` uses
  the toolchain pinned in `rust-toolchain.toml`.

With Docker installed, no other setup is needed: the script pulls the pinned
generator image. Alternatively, run the scripts in the image built from
`Dockerfile-autogen-models`, which has the npm wrapper, Java and the pinned
Rust toolchain:

```sh
# From the root of the repository
docker build -t ga4gh-sdk-autogen-models -f Dockerfile-autogen-models .
docker run -it --rm -v "$(pwd):/app" -w /app ga4gh-sdk-autogen-models \
    bash ./utils/build_models_wrapper.sh
```

## Usage

### Regenerate the models of this repository

From the root of the repository:

```sh
# All APIs
./utils/build_models_wrapper.sh
# Only some APIs
./utils/build_models_wrapper.sh wes tes
git status lib/src/clients
```

To move an API to a new spec version, change its entry in `SPECS` in
`utils/build_models_wrapper.sh`, run the wrapper for it, update its patch if
it no longer applies (see below), and adapt the client in
`lib/src/clients/<api>/mod.rs` to the changed models.

### Generate models for a single spec, or for another crate

```sh
./utils/build_models.sh [-m MODULE_PATH] [-p PATCH_FILE] [-n] \
    <OPENAPI_SPEC_PATH> <API_NAME> <DESTINATION_DIR> [-- GENERATOR_ARGS...]
```

- `OPENAPI_SPEC_PATH`: URL or local path of the spec.
- `API_NAME`: name of the API; used for the default module path.
- `DESTINATION_DIR`: the folder that receives `models/`. An existing
  `models/` folder in it is replaced.
- `-m MODULE_PATH`: Rust path of the `models` module that the generated files
  import, by default `crate::clients::<API_NAME>::models`. Set it when the
  models live elsewhere, for example in a crate that builds on this SDK:

  ```sh
  ./utils/build_models.sh -m crate::api::models \
      https://example.org/my-api.openapi.yaml myapi src/api/
  ```

- `-p PATCH_FILE`: a patch with hand edits, applied after generation.
- `-n`: don't run `rustfmt`; used to write patches, which apply to the
  unformatted output.
- `GENERATOR_ARGS`: extra arguments for `openapi-generator-cli generate`,
  for example `--skip-validate-spec` (needed for DRS 1.4.0).
- `OPENAPI_GENERATOR_VERSION` (environment): generator version, default
  `7.9.0`.

## Hand edits

Each API has at most one patch, `utils/patches/<api>.patch`, which the wrapper
passes to `build_models.sh`. The script applies it to the generated files after
fixing their `use` statements and before formatting them. A patch is a unified
diff with paths `a/models/<file>` and `b/models/<file>`; text above the first
`---` line explains the edits.

To change a patch:

```sh
api=wes
spec=https://raw.githubusercontent.com/ga4gh/workflow-execution-service-schemas/1.1.0/openapi/workflow_execution_service.openapi.yaml
work=$(mktemp -d)
# 1. Generate without the patch and without formatting
./utils/build_models.sh -n "$spec" "$api" "$work/a"
cp -r "$work/a" "$work/b"
# 2. Edit the files in $work/b/models, then write the patch
(cd "$work" && diff -ruN a/models b/models) > "utils/patches/$api.patch"
# 3. Add the explanation at the top of the patch and regenerate
./utils/build_models_wrapper.sh "$api"
rm -rf "$work"
```

Keep patches small: prefer fixing the spec upstream, or a type outside
`models/`, over a large patch.
