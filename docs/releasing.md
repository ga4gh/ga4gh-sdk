# Releasing

A version tag `v<version>` on `main` starts the [Release workflow](../.github/workflows/release.yml).
For now, it publishes the Python bindings to PyPI. It doesn't create a GitHub
release; that comes with the CLI binaries
([#91](https://github.com/ga4gh/ga4gh-sdk/issues/91)).

## What the workflow does

1. **Prepare release** checks that the tag is `v` followed by the workspace
   version in the root `Cargo.toml` (for example `v0.2.0` or `v0.2.0-rc.1`)
   and that the tagged commit is on `main`. It then chooses the package index.
2. **Release wheels** ([`wheels.yml`](../.github/workflows/wheels.yml)) builds
   one `abi3` wheel per platform with [maturin](https://www.maturin.rs/) and an
   sdist, and imports each wheel and the sdist in a fresh environment. Each
   wheel covers CPython 3.11 and newer:

   | Platform | Wheel tag |
   | --- | --- |
   | Linux x86-64 | `manylinux_2_28_x86_64` |
   | Linux aarch64 | `manylinux_2_28_aarch64` |
   | macOS x86-64 | `macosx_*_x86_64` |
   | macOS arm64 | `macosx_*_arm64` |
   | Windows x86-64 | `win_amd64` |

   Python wheels use rustls with native system certificate roots, without
   bundling OpenSSL. Elsewhere (for example musl-based Linux such as Alpine,
   or glibc older than 2.28), pip builds from the sdist, which needs the Rust
   toolchain (see the [Python guide](../python/README.md#prerequisites)).
   CI runs the same build on every pull request, without publishing.
3. **Publish package** uploads all files with
   [trusted publishing](https://docs.pypi.org/trusted-publishers/): there is no
   API token, PyPI accepts the upload because it comes from this workflow and
   environment. The upload also carries
   [attestations](https://docs.pypi.org/attestations/) of where each file was
   built.

The Python version comes from the workspace version; maturin converts it to
PEP 440, so `0.2.0-rc.1` becomes `0.2.0rc1` on PyPI. Files on PyPI can't be
replaced: a failed or wrong upload needs a new version.

The maturin version is pinned in `wheels.yml` and in the `python` job of
`ci.yml`; change both together.

## Package index

| Repository | Default index | GitHub environment |
| --- | --- | --- |
| `ga4gh/ga4gh-sdk` | PyPI | `pypi` |
| A fork | TestPyPI | `testpypi` |

The repository variable `PYPI_INDEX` (`pypi` or `testpypi`) overrides the
default, for example to try a release of `ga4gh/ga4gh-sdk` on TestPyPI first.
For an upstream TestPyPI rehearsal, use a distinct pre-release workspace
version and tag, such as `0.1.0-dev.1` and `v0.1.0-dev.1`. The workflow rejects
an upstream TestPyPI upload of a stable version. An immutable tag cannot later
be reused to publish the same version to PyPI. The run shows a warning; delete
the variable afterwards, or the next tag goes to TestPyPI, too.

## One-time setup

The `ga4gh/ga4gh-sdk` repository already has `pypi` and `testpypi`
environments limited to `v*` tags. Both require approval by
`pavelnikonorov` or `uniqueg` and prevent self-approval; neither needs secrets.
Two tag rulesets allow only those two users to create `v*` tags and prevent
updates or deletion. Verify these protections before each release.

For the demonstrator release, the
[`GENXT` PyPI organization](https://pypi.org/org/GENXT/) will temporarily own
the `ga4gh-sdk` project. An authorized GENXT Owner or Manager should:

1. In **Your organizations → GENXT → Projects**, create the `ga4gh-sdk`
   project before the first release. PyPI [allows organizations to create a
   project](https://docs.pypi.org/organization-accounts/actions/project-actions/#create-a-project)
   without uploading a distribution. Confirm that the new project belongs to
   GENXT. Its name remains `ga4gh-sdk`; the organization's uppercase spelling
   does not change the package or Python import name.
2. In that project's **Publishing** settings, add a
   [GitHub trusted publisher](https://docs.pypi.org/trusted-publishers/adding-a-publisher/)
   with GitHub owner `ga4gh`, repository `ga4gh-sdk`, workflow `release.yml`,
   and environment `pypi`. These fields identify the GitHub repository and do
   not change when PyPI project ownership changes. No API token is needed.

TestPyPI has separate accounts and project ownership. For the fork dry run
below, use a distinct project name on TestPyPI. If also rehearsing the upstream
release there, configure its `testpypi` publisher under whichever TestPyPI
account owns that `ga4gh-sdk` project; do not assume the PyPI GENXT
organization exists on TestPyPI.

After the demonstrator presentation, transfer the PyPI project from GENXT to
the [`ga4gh` PyPI account](https://pypi.org/user/ga4gh/). In the project's
Collaborators settings, invite `ga4gh` as an external **Owner** and have that
account accept. Verify that `ga4gh` can manage the project and its trusted
publisher. Then use **Settings → Remove project** to
[remove it from GENXT](https://docs.pypi.org/organization-accounts/actions/project-actions/#remove-project-from-organization);
this removes GENXT team permissions while retaining individual Owner roles.
Verify the final owner on PyPI. Apply the same steps on TestPyPI only if GENXT
owns the upstream project there. This organizational ownership is temporary
until the transfer is confirmed.

## Making a release

1. Set the version in `[workspace.package]` of the root `Cargo.toml`, run
   `cargo update --workspace` and merge that change into `main`.
2. Tag the merge commit and push the tag:

   ```sh
   git fetch origin
   git tag -a v0.2.0 -m v0.2.0 <commit>
   git push origin v0.2.0
   ```

3. Approve the deployment to `pypi` in the workflow run once the wheels are
   built.

## Dry run in a fork

Try changes to the workflows in a fork before tagging `ga4gh/ga4gh-sdk`. A fork
publishes under its own name, because TestPyPI ties a project to its first
publisher and `ga4gh-sdk` is kept for this repository:

1. In the fork, change `name` in `python/pyproject.toml` to
   `ga4gh-sdk-<your-user>`, and create the environment `testpypi`.
2. On TestPyPI, add a pending trusted publisher for `ga4gh-sdk-<your-user>`
   with the fork's owner and repository, workflow `release.yml` and
   environment `testpypi`.
3. Set the workspace version to a version that is not yet on TestPyPI, for
   example `0.1.0-dev.1`, and run `cargo update --workspace`. Commit the
   version, `Cargo.lock` and the package name on the fork's `main`, tag that
   commit with `v0.1.0-dev.1` and push the tag to the fork.
4. Check that the run publishes to TestPyPI, then install from there:

   ```sh
   python -m pip install --index-url https://test.pypi.org/simple/ ga4gh-sdk-<your-user>==0.1.0.dev1
   python -c 'import ga4gh_sdk; print(ga4gh_sdk.TES.__name__)'
   ```

5. Delete the test tag in the fork afterwards.
