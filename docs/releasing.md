# Releasing

A version tag `v<version>` on `main` starts the [Release workflow](../.github/workflows/release.yml).
For now, it publishes the Python bindings to PyPI and the
[CLI image](container-image.md) to the GitHub Container Registry. It doesn't
create a GitHub release; that comes with the CLI binaries
([#91](https://github.com/ga4gh/ga4gh-sdk/issues/91)).

## What the workflow does

1. **Prepare release** checks that the tag is `v` followed by the workspace
   version in the root `Cargo.toml` (for example `v0.2.0` or `v0.2.0-rc.1`)
   and that the tagged commit is on `main`. It then chooses the package index.
2. **Release wheels** ([`wheels.yml`](../.github/workflows/wheels.yml)) builds
   one `abi3` wheel per platform with [maturin](https://www.maturin.rs/) and an
   sdist, checks that every declared license file is in the sdist, and imports
   each artifact in a fresh environment. Each wheel covers CPython 3.11 and
   newer:

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
3. **Release image** ([`image.yml`](../.github/workflows/image.yml)) builds
   the [CLI image](container-image.md) natively for `linux/amd64` and
   `linux/arm64`, checks each with `tests/run-image-tests.sh` (on
   `linux/amd64` also against Funnel) and pushes each platform image by
   digest, without a tag. The push rebuilds the checked image from the
   builder's layer cache, with the source and revision labels and
   reproducible timestamps, and runs `--version` on the pushed digest. CI runs the same build and checks on every pull
   request, without pushing.
4. **Publish package** waits for the wheels and the image, so that nothing
   reaches PyPI unless every artefact is built. It uploads all files with
   [trusted publishing](https://docs.pypi.org/trusted-publishers/): there is no
   API token, PyPI accepts the upload because it comes from this workflow and
   environment. The upload also carries
   [attestations](https://docs.pypi.org/attestations/) of where each file was
   built.
5. **Publish image** runs after the package is on the index. It combines the
   two platform images into one multi-platform image, tags it with the
   version and records a build provenance attestation for it (see
   [CLI image](#cli-image)).

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

## CLI image

| Repository | Image |
| --- | --- |
| `ga4gh/ga4gh-sdk` | `ghcr.io/ga4gh/ga4gh-cli` |
| A fork | `ghcr.io/<fork owner>/ga4gh-cli` |

The workflow pushes with the job's `GITHUB_TOKEN`; it needs no secret and no
environment. The tag is the workspace version, such as `0.2.0` or
`0.2.0-rc.1` (a `+` of build metadata becomes `_`, which OCI tags allow).
There is no `latest` tag. An upstream TestPyPI rehearsal (`PYPI_INDEX` set
to `testpypi` in `ga4gh/ga4gh-sdk`) pushes the platform images by digest
but tags no image, and **Publish image** is skipped. Unlike files on PyPI, a tag can be moved: a re-run
of **Publish image** points the version tag at the images of that run, and
earlier digests stay available.

On the first push, GitHub creates the `ga4gh-cli` package, connected to this
repository. Check after the first release that the package is public (the
package's settings on GitHub, **Change visibility**) and that it inherits
access from the repository; organization settings can restrict who creates
public packages.

## One-time setup

The `ga4gh/ga4gh-sdk` repository already has `pypi` and `testpypi`
environments limited to `v*` tags. Both require approval by
`pavelnikonorov` or `uniqueg` and prevent self-approval; neither needs secrets.
Two tag rulesets allow only those two users to create `v*` tags and prevent
updates or deletion. Verify these protections before each release.

For the demonstrator release, the
[`pavelnikonorov` PyPI account](https://pypi.org/user/pavelnikonorov/) will
temporarily own the `ga4gh-sdk` project. In that account's **Publishing**
settings, a [pending GitHub trusted publisher](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
is registered with project name `ga4gh-sdk`, GitHub owner `ga4gh`, repository
`ga4gh-sdk`, workflow `release.yml`, and environment `pypi`. Verify these
settings before the first release. The project name matches
`python/pyproject.toml`; Python imports use `ga4gh_sdk`. The GitHub owner
remains `ga4gh` after a PyPI ownership transfer. No API token is needed.
A pending publisher neither creates the project nor reserves its name. The
first successful upload creates the project under the account that registered
the pending publisher; check name availability again before releasing.

TestPyPI has separate accounts and project ownership. For the fork dry run
below, use a distinct project name on TestPyPI. If also rehearsing the upstream
release there, configure its `testpypi` publisher under whichever TestPyPI
account owns that `ga4gh-sdk` project. PyPI and TestPyPI accounts are separate.

After the demonstrator presentation, transfer the PyPI project to the
[`ga4gh` PyPI account](https://pypi.org/user/ga4gh/). In the project's
Collaborators settings, invite `ga4gh` as **Owner** and have that account
accept. Verify that `ga4gh` can manage the project and its trusted publisher.
Then remove `pavelnikonorov` as Owner if sole `ga4gh` ownership is desired.
Verify the final owner on PyPI; repeat on TestPyPI only if an upstream project
was created there. Personal ownership is temporary until the transfer is
confirmed.

## Making a release

1. Set the version in `[workspace.package]` of the root `Cargo.toml`, run
   `cargo update --workspace` and merge that change into `main`.
2. Tag the merge commit and push the tag:

   ```sh
   git fetch origin
   git tag -a v0.2.0 -m v0.2.0 <commit>
   git push origin v0.2.0
   ```

3. Approve the deployment to `pypi` in the workflow run once the wheels and
   the image are built.
4. Check the image tag and its attestation:

   ```sh
   docker buildx imagetools inspect ghcr.io/ga4gh/ga4gh-cli:0.2.0
   gh attestation verify oci://ghcr.io/ga4gh/ga4gh-cli:0.2.0 --repo ga4gh/ga4gh-sdk
   ```

## Dry run in a fork

Try changes to the workflows in a fork before tagging `ga4gh/ga4gh-sdk`. Use
a distinct, available test-only distribution name that explains its purpose,
such as `ga4gh-sdk-release-rehearsal`. The TestPyPI publisher and the name in
`python/pyproject.toml` must match exactly:

1. Before making test changes, [sync the fork's `main` with upstream
   `main`](https://docs.github.com/en/pull-requests/how-tos/work-with-forks/syncing-a-fork)
   (`gh repo sync YOUR_USERNAME/ga4gh-sdk -b main`) and verify it is not
   behind. Check again immediately before tagging. If the fork already has
   test commits, bring upstream changes into it without discarding those
   commits or an existing test tag. Resolve any sync conflict; do not use
   `gh repo sync --force`.
2. In the fork, change `name` in `python/pyproject.toml` to the chosen test
   name, and create the environment `testpypi`.
3. On TestPyPI, add a pending trusted publisher for the same test name
   with the fork's owner and repository, workflow `release.yml` and
   environment `testpypi`.
4. Set the workspace version to a version that is not yet on TestPyPI, for
   example `0.1.0-dev.1`, and run `cargo update --workspace`. Commit the
   version, `Cargo.lock` and the package name on the fork's `main`, tag that
   commit with `v0.1.0-dev.1` and push the tag to the fork.
5. Check that the run publishes to TestPyPI, then install from there:

   ```sh
   python -m pip install --index-url https://test.pypi.org/simple/ ga4gh-sdk-release-rehearsal==0.1.0.dev1
   python -c 'import ga4gh_sdk; print(ga4gh_sdk.TES.__name__)'
   ```

   Substitute your chosen test name in the install command if it differs from
   this example.
6. Check that the image is at `ghcr.io/<fork owner>/ga4gh-cli:0.1.0-dev.1`
   and runs: `docker run --rm ghcr.io/<fork owner>/ga4gh-cli:0.1.0-dev.1 --version`.
   The package is private in a fork unless you make it public; log in with
   `gh auth token | docker login ghcr.io -u <user> --password-stdin` first.
7. Keep the test tag until the run and package can be reviewed; delete it from
   the fork later if it is no longer needed, together with the fork's
   `ga4gh-cli` package.
