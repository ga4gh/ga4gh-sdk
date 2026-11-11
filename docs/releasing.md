# Releasing

A version tag `v<version>` on `main` starts the [Release workflow](../.github/workflows/release.yml).
It publishes the Python bindings to PyPI and creates a draft GitHub release
with the CLI binaries, which a maintainer completes and publishes.

## What the workflow does

1. **Prepare release** checks that the tag is `v` followed by the workspace
   version in the root `Cargo.toml` (for example `v0.2.0` or `v0.2.0-rc.1`)
   and that the tagged commit is on `main`. It then chooses the package index
   and whether the GitHub release is a pre-release: every version before
   `1.0.0` is, and so is every version with a pre-release part such as
   `1.1.0-rc.1`.
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

   The Linux wheels bundle the OpenSSL library that the HTTP client links
   against. Elsewhere (for example musl-based Linux such as Alpine, or glibc
   older than 2.28), pip builds from the sdist, which needs the Rust toolchain
   and the OpenSSL headers (see the [Python guide](../python/README.md#prerequisites)).
   CI runs the same build on every pull request, without publishing.
3. **Publish package** uploads all files with
   [trusted publishing](https://docs.pypi.org/trusted-publishers/): there is no
   API token, PyPI accepts the upload because it comes from this workflow and
   environment. The upload also carries
   [attestations](https://docs.pypi.org/attestations/) of where each file was
   built.
4. **Release binaries** ([`binaries.yml`](../.github/workflows/binaries.yml))
   builds `ga4gh-cli` natively on one runner per platform, runs `--help` and
   `--version` on it, and packs it with `LICENSE` and `README.md` into
   `ga4gh-cli-<version>-<target>.tar.gz` (`.zip` on Windows):

   | Platform | Target | Runner |
   | --- | --- | --- |
   | Linux x86-64 | `x86_64-unknown-linux-gnu` | `ubuntu-22.04` |
   | Linux aarch64 | `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` |
   | macOS x86-64 | `x86_64-apple-darwin` | `macos-15-intel` |
   | macOS arm64 | `aarch64-apple-darwin` | `macos-15` |
   | Windows x86-64 | `x86_64-pc-windows-msvc` | `windows-2025` |
   | Windows arm64 | `aarch64-pc-windows-msvc` | `windows-11-arm` |

   The Linux binaries link the system's OpenSSL 3 and glibc dynamically. They
   are built on Ubuntu 22.04, and the job fails if a binary needs a glibc
   symbol newer than 2.34, the floor that the README documents (RHEL 9). The
   binaries aren't code-signed or notarised. CI runs the same build on every
   pull request, without publishing. This job runs in parallel with the
   wheels; **Publish package** waits for both, so that nothing reaches PyPI
   unless every artefact of the release is built. The Windows binaries link
   the C runtime statically and need no Visual C++ Redistributable.
5. **GitHub release** runs after the package is published. It writes
   `SHA256SUMS` for the archives, records
   [build provenance attestations](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations)
   for them, and creates a **draft** release for the tag, flagged as a
   pre-release as chosen in step 1, with the archives, `SHA256SUMS` and
   GitHub's generated notes. A re-run replaces the files of the draft; it
   stops if the release is already published or if it can't check whether a
   release exists.

If a run fails after **Publish package** has uploaded the files, use **Re-run
failed jobs**, not **Re-run all jobs**: PyPI refuses files it already has, so
a full re-run fails at the upload and never reaches the GitHub release. If the
deployment to `pypi` is rejected, the run creates no GitHub release.

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
The run then shows a warning. Delete the variable afterwards, or the next tag
goes to TestPyPI, too.

## One-time setup

Maintainers of `ga4gh/ga4gh-sdk` set this up once before the first tag:

1. On PyPI, add a pending
   [trusted publisher](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
   for the project `ga4gh-sdk` with owner `ga4gh`, repository `ga4gh-sdk`,
   workflow `release.yml` and environment `pypi`. Do the same on TestPyPI with
   environment `testpypi`, so that the name is reserved there for this
   repository, too.
2. In the repository settings, create the environments `pypi` and `testpypi`.
   Limit them to tags matching `v*` (deployment branches and tags) and add
   required reviewers, so that every upload needs a maintainer's approval.
   Neither environment needs secrets. The CLI binaries and the GitHub release
   need no environment and no secret either; that job gets write access to
   the repository contents from the workflow's own token.
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

3. Approve the deployment to `pypi` in the workflow run once the wheels and
   the CLI binaries are built.
4. When the run has finished, open the draft release, add the curated notes
   above GitHub's generated ones, check that it lists the six CLI archives and
   `SHA256SUMS`, and publish it. A draft is visible to maintainers only and can
   be edited or deleted; once published, its files may no longer be
   replaceable (for example with immutable releases).

## Dry run in a fork

Try changes to the workflows in a fork before tagging `ga4gh/ga4gh-sdk`. The
fork needs GitHub Actions enabled. A fork publishes under its own name, because
TestPyPI ties a project to its first publisher and `ga4gh-sdk` is kept for this
repository:

1. In the fork, change `name` in `python/pyproject.toml` to
   `ga4gh-sdk-<your-user>`, and create the environment `testpypi`.
2. On TestPyPI, add a pending trusted publisher for `ga4gh-sdk-<your-user>`
   with the fork's owner and repository, workflow `release.yml` and
   environment `testpypi`.
3. Set the workspace version to a version that is not yet on TestPyPI, for
   example `0.1.0-dev.1`, and run `cargo update --workspace`. Commit the
   version, `Cargo.lock` and the package name on the fork's `main`, tag that
   commit with `v0.1.0-dev.1` and push the tag to the fork.
4. Check that the run publishes to TestPyPI (the environment link in the run
   still points to `ga4gh-sdk`), then install from there:

   ```sh
   python -m pip install --index-url https://test.pypi.org/simple/ ga4gh-sdk-<your-user>==0.1.0.dev1
   python -c 'import GA4GH; print(GA4GH.TES.__name__)'
   ```

5. Check the draft release in the fork: six CLI archives and `SHA256SUMS`,
   flagged as a pre-release. Download one archive, check it with
   `sha256sum --check --ignore-missing SHA256SUMS` and
   `gh attestation verify <archive> --repo <fork-owner>/ga4gh-sdk`, and run
   `ga4gh-cli --version`.
6. Delete the draft release and the test tag in the fork afterwards.
