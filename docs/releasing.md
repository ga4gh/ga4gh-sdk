# Releasing

A version tag `v<version>` on `main` starts the [Release workflow](../.github/workflows/release.yml).
It publishes the Python bindings to PyPI. The CLI binaries
([#91](https://github.com/ga4gh/ga4gh-sdk/issues/91)) are not part of it yet.

## What the workflow does

1. **Prepare release** checks that the tag is `v` followed by the workspace
   version in the root `Cargo.toml` (for example `v0.2.0` or `v0.2.0-rc.1`)
   and that the tagged commit is on `main`. It then chooses the package index.
2. **Wheels** ([`wheels.yml`](../.github/workflows/wheels.yml)) builds one
   `abi3` wheel per platform with [maturin](https://www.maturin.rs/) and an
   sdist. Each wheel covers CPython 3.11 and newer:

   | Platform | Wheel tag |
   | --- | --- |
   | Linux x86-64 | `manylinux_2_28_x86_64` |
   | Linux aarch64 | `manylinux_2_28_aarch64` |
   | macOS x86-64 | `macosx_*_x86_64` |
   | macOS arm64 | `macosx_*_arm64` |
   | Windows x86-64 | `win_amd64` |

   The Linux wheels bundle the OpenSSL library that the HTTP client links
   against. On other platforms, pip builds from the sdist, which needs the
   Rust toolchain. CI runs the same build on every pull request, without
   publishing.
3. **Publish to PyPI** (or **Publish to TestPyPI**) uploads all files with
   [trusted publishing](https://docs.pypi.org/trusted-publishers/): there is no
   API token, PyPI accepts the upload because it comes from this workflow and
   environment. The upload also carries
   [attestations](https://docs.pypi.org/attestations/) of where each file was
   built.

The Python version comes from the workspace version; maturin converts it to
PEP 440, so `0.2.0-rc.1` becomes `0.2.0rc1` on PyPI. Files on PyPI can't be
replaced: a failed or wrong upload needs a new version.

## Package index

| Repository | Default index | GitHub environment |
| --- | --- | --- |
| `ga4gh/ga4gh-sdk` | PyPI | `pypi` |
| A fork | TestPyPI | `testpypi` |

The repository variable `PYPI_INDEX` (`pypi` or `testpypi`) overrides the
default, for example to try a release of `ga4gh/ga4gh-sdk` on TestPyPI first.

## One-time setup

Maintainers of `ga4gh/ga4gh-sdk` set this up once before the first tag:

1. On PyPI, add a pending
   [trusted publisher](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
   for the project `ga4gh-sdk` with owner `ga4gh`, repository `ga4gh-sdk`,
   workflow `release.yml` and environment `pypi`. Do the same on TestPyPI with
   environment `testpypi` if releases should be tried there.
2. In the repository settings, create the environment `pypi`. Limit it to
   tags matching `v*` (deployment branches and tags) and add required
   reviewers, so that every upload needs a maintainer's approval. Create
   `testpypi` the same way, if needed. Neither environment needs secrets.
3. Protect `v*` tags with a tag ruleset, so that only maintainers can create
   them and nobody can move or delete them.

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

Try changes to the workflows in a fork before tagging `ga4gh/ga4gh-sdk`:

1. In the fork, create the environment `testpypi`.
2. On TestPyPI, add a pending trusted publisher for `ga4gh-sdk` with the fork's
   owner and repository, workflow `release.yml` and environment `testpypi`.
   TestPyPI ties a project to its first publisher; if `ga4gh-sdk` already
   exists there, change `name` in `python/pyproject.toml` in the fork, for
   example to `ga4gh-sdk-<your-user>`.
3. Set the workspace version to a version that is not yet on TestPyPI, for
   example `0.1.0-dev.1`, commit it on the fork's `main`, tag that commit with
   `v0.1.0-dev.1` and push the tag to the fork.
4. Check that the run publishes to TestPyPI, then install from there:

   ```sh
   python -m pip install --index-url https://test.pypi.org/simple/ ga4gh-sdk==0.1.0.dev1
   python -c 'import GA4GH; print(GA4GH.TES.__name__)'
   ```

5. Delete the test tag in the fork afterwards.
