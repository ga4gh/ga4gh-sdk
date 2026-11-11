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

For the demonstrator release, `pavelnikonorov` will temporarily own the
`ga4gh-sdk` project on PyPI. Before the first tag, sign in to that PyPI account
and add a pending
[trusted publisher](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
for project `ga4gh-sdk`: GitHub owner `ga4gh`, repository `ga4gh-sdk`, workflow
`release.yml`, environment `pypi`. The GitHub owner is the repository's
organization, regardless of which PyPI account owns the project. If rehearsing
the upstream release on TestPyPI, configure its pending publisher separately
under the `pavelnikonorov` TestPyPI account, with environment `testpypi`.
PyPI and TestPyPI have separate accounts and project ownership. A pending
publisher does not reserve the project name; each index creates and assigns
the project only after its first successful upload.

After the demonstrator presentation, transfer each published project from
`pavelnikonorov` to `ga4gh`: invite `ga4gh` as an **Owner** in the project's
Collaborators settings, have that account accept, and verify that it can manage
the project and its trusted publisher. Only then remove `pavelnikonorov` as an
Owner if the team wants sole ownership under `ga4gh`. Repeat on TestPyPI if the
upstream project was created there. The personal-account ownership remains
temporary until the transfer is confirmed.

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
