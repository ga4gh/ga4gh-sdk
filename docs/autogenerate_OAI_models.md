## Motivation

The models in `lib/src/clients/<api>/models/` and versioned directories
such as `lib/src/clients/trs/wire/v2_0_1/models/` are generated from the
[OpenAPI](https://www.openapis.org/) specifications of GA4GH APIs. They
represent request and response bodies. Regenerate them when a pinned source,
template, or patch changes, or generate a model set for an additional contract.

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
| `registry` | [Service Registry 1.0.0](https://github.com/ga4gh-discovery/ga4gh-service-registry/tree/1.0.0) | – |
| `tes` | [TES 1.1](https://github.com/ga4gh/task-execution-schemas/tree/v1.1) | `utils/patches/tes.patch` |
| `drs` | [DRS 1.4.0](https://github.com/ga4gh/data-repository-service-schemas/tree/drs-1.4.0) | `utils/patches/drs.patch` |
| `trs` versioned wire models, re-exported as `trs::models` | [TRS 2.0.1](https://github.com/ga4gh/tool-registry-service-schemas/tree/2.0.1) | `utils/patches/trs.patch` |
| `wes` | [WES 1.1.0](https://github.com/ga4gh/workflow-execution-service-schemas/tree/1.1.0) | `utils/patches/wes.patch` |

TRS also supports the [2.0.2 contract](https://github.com/ga4gh/tool-registry-service-schemas/tree/2.0.2),
whose only additional generated model field is optional
`ToolVersion.description`. The 2.0.2 spec still declares `info.version: 2.1.0`;
the client does not treat a server declaration of `2.1.0` as 2.0.2. The
hand-written 2.0.2 adapter checks the new field without generating a duplicate
model set. `utils/build_models_wrapper.sh trs` regenerates the 2.0.1 models in
`lib/src/clients/trs/wire/v2_0_1/models/` with `-m`; the TRS module re-exports
them as `trs::models`. See the [model layout decision](api_compatibility.md#model-layout)
and the
[profile extension steps](api_compatibility.md#adding-a-profile) before adding
a version. Treat every `models/` directory as generated code.

The models are generated with OpenAPI Generator 7.9.0 and formatted with
`rustfmt`. Running the wrapper on a clean checkout must not change any file in
`lib/src/clients`. If it does, a spec, the generator or a patch has changed.
No CI job checks this yet; run the wrapper and `git status lib/src/clients`
after changing a spec, the template or a patch.

## Model template

The generator uses `utils/templates/rust/model.mustache`, a copy of the
`model.mustache` of the Rust generator of OpenAPI Generator 7.9.0 with these
edits, so that the models keep what their spec doesn't define:

- Every struct gets a field `extra: serde_json::Map<String, serde_json::Value>`
  with `#[serde(flatten)]`, which takes the fields that the spec doesn't
  define; `new` sets it to an empty map.
- Every string enum gets a last variant `Unrecognized(String)` with
  `#[serde(untagged)]`, which takes the values that the spec doesn't define.
  The enums are no longer `Copy`. Enums of a single property (for example
  the DRS `access_method::Type`) also get `Display`, like the other enums.

Patches adapt single models: `TesState` and the WES `State` read unknown states
as `Unknown` instead, which both specs define as the safe default, and stay
`Copy`; the TRS `FileWrapperImageType` reads known values of its two enums
before unknown ones (see the patches for details). Integer enums and enums
with a discriminator are unchanged; none of the pinned specs has them.

When you change the generator version, compare the template with the
`model.mustache` of the new version (`openapi-generator-cli author template -g
rust`) and carry over the edits listed above.

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
# TRS wire models (2.0.1, re-exported through trs::models)
./utils/build_models_wrapper.sh trs
git status lib/src/clients
```

To add a model set while retaining supported versions, add an entry to `SPECS`
in `utils/build_models_wrapper.sh` with the API name, pinned source, a separate
destination, and its Rust module path. Keep the retained entries. The wrapper
regenerates every matching entry when given that API name. Follow the
[profile extension steps](api_compatibility.md#adding-a-profile) to integrate
the generated types with client behavior and tests.

Replace an existing entry only when intentionally changing that model set's
contract. Check the effect on supported servers and public Rust types first.
Run the wrapper for the affected API, update its patch if needed (see below),
and adapt the client and migration guidance to the changed models.

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
  `7.9.0`. The [model template](#model-template) is a copy of the 7.9.0
  template; check it before you use another version.

The script always uses the model template, also for other crates, so their
models keep unknown fields and values too. Their crate needs `serde_json` and
`serde` 1.0.181 or later.

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
