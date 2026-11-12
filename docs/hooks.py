"""MkDocs hooks: the pages of the site are Markdown files of the repository.

The guides live next to the code they describe (`README.md`, `cli/README.md`,
`python/README.md`, `docs/`, ...) and link to each other and to code with
paths relative to the file. So that these links work on GitHub and on the
site alike, the site is built from the files at their paths in the
repository instead of from the files of `docs_dir`:

- `on_files` collects the Markdown files that `nav` lists (paths in the
  repository; `docs/` is dropped from their URLs), the images of `images/`
  and the Rust API reference of `cargo doc` (`target/doc/<crate>/`, served
  as `rust/<crate>/`).
- `on_page_markdown` points relative links to other files of the repository,
  such as source files or guides that are not in `nav`, to the files on
  GitHub, at the commit that Read the Docs builds (`main` elsewhere). Links
  to files that Git doesn't track stay as they are, so the strict build
  fails on them.
"""

from __future__ import annotations

import logging
import os
from pathlib import Path
import posixpath
import re
import subprocess
from typing import Any
from urllib.parse import urlsplit

from mkdocs.config.defaults import MkDocsConfig
from mkdocs.structure.files import File, Files
from mkdocs.structure.nav import Link, Navigation

ROOT = Path(__file__).resolve().parents[1]
REPO_URL = "https://github.com/ga4gh/ga4gh-sdk"
# Static files served at their paths in the repository
ASSETS = ("images",)
# Crates of the Rust API reference (`cargo doc --no-deps -p ...`)
CRATES = ("ga4gh_sdk", "ga4gh_cli")
RUSTDOC_PREFIX = "rust"
# Loggers below `mkdocs` count towards the warnings of `--strict`
log = logging.getLogger("mkdocs.hooks.repo_pages")

# `[text](target "title")` and `![alt](target)`; `<target>` too
INLINE_LINK = re.compile(r"(?P<head>\]\(\s*)(?P<target><[^>]*>|[^)\s]+)")
# `[label]: target "title"`, but not footnotes (`[^1]: text`)
REFERENCE = re.compile(r"^(?P<head> {0,3}\[(?!\^)[^\]]+\]:\s*)(?P<target><[^>]*>|\S+)")
# Inline code spans, which are left as they are
CODE_SPAN = re.compile(r"(`+).*?\1")
FENCE = re.compile(r"^\s*(```+|~~~+)")


def _nav_paths(nav: Any) -> list[str]:
    """List the paths in the `nav` configuration.

    Args:
        nav: `nav` configuration, or a part of it.

    Returns:
        Paths of the entries, in order; URLs included.
    """
    if isinstance(nav, str):
        return [nav]
    if isinstance(nav, dict):
        return [path for value in nav.values() for path in _nav_paths(value)]
    if isinstance(nav, list):
        return [path for item in nav for path in _nav_paths(item)]
    return []


def _tracked_paths() -> set[str] | None:
    """List the files that Git tracks, with their directories.

    Links to other files, such as build output, are not rewritten, so that
    the strict build reports them.

    Returns:
        Paths relative to the repository root, or `None` without Git.
    """
    try:
        result = subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "-z"],
            capture_output=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        log.info("Git is not available; links to untracked files are rewritten too")
        return None
    paths = set()
    for path in result.stdout.decode().split("\0"):
        while path:
            paths.add(path)
            path = posixpath.dirname(path)
    return paths


TRACKED = _tracked_paths()


def _rustdoc_dir() -> Path:
    """Locate the output of `cargo doc`.

    Returns:
        `doc/` of the Cargo target directory.
    """
    target = os.environ.get("CARGO_TARGET_DIR")
    return (Path(target) if target else ROOT / "target") / "doc"


class _StaticFile(File):
    """A file that is copied as it is, even Markdown (e.g., licence texts)."""

    def is_documentation_page(self) -> bool:
        """Tell whether the file is a page to render.

        Returns:
            `False`, always.
        """
        return False


def _add(files: Files, path: str, config: MkDocsConfig, source: Path | None = None) -> None:
    """Add a file to the site.

    Args:
        files: Files of the site.
        path: Path of the file in the site's sources: in the repository, or
            below `rust/` for the Rust API reference.
        config: MkDocs configuration.
        source: The file, if it isn't `path` in the repository.
    """
    kind = File if source is None else _StaticFile
    file = kind(path, str(ROOT), config.site_dir, config.use_directory_urls)
    if source is not None:
        file.abs_src_path = str(source)
    if path.startswith("docs/"):
        file.dest_uri = file.dest_uri.removeprefix("docs/")
    files.append(file)


def on_files(files: Files, config: MkDocsConfig) -> Files:
    """Replace the files of `docs_dir` with the files of the repository.

    Args:
        files: Files of `docs_dir` and of the theme.
        config: MkDocs configuration.

    Returns:
        The theme's files, the pages listed in `nav`, assets and the Rust
        API reference.

    Raises:
        FileNotFoundError: A Markdown file listed in `nav` is missing.
    """
    # The theme's files (styles, scripts, ...) stay
    docs_dir = os.path.normpath(config.docs_dir)
    site = Files([file for file in files if os.path.normpath(file.src_dir or "") != docs_dir])
    for path in _nav_paths(config.nav):
        if not path.endswith(".md") or urlsplit(path).scheme:
            continue
        if not (ROOT / path).is_file():
            raise FileNotFoundError(f"{path} is listed in 'nav' but doesn't exist")
        _add(site, path, config)
    for directory in ASSETS:
        for asset in sorted((ROOT / directory).rglob("*")):
            if asset.is_file():
                _add(site, asset.relative_to(ROOT).as_posix(), config)
    rustdoc = _rustdoc_dir()
    if not all((rustdoc / crate / "index.html").is_file() for crate in CRATES):
        log.warning(
            "The Rust API reference is missing; run "
            "'cargo doc --no-deps --locked -p ga4gh-lib -p ga4gh-cli' first"
        )
        return site
    # The crates' pages and the files they share (search index, styles,
    # source pages, ...; about 20 MB); `.lock` is Cargo's
    for item in sorted(rustdoc.rglob("*")):
        relative = item.relative_to(rustdoc).as_posix()
        if item.is_file() and relative != ".lock":
            _add(site, f"{RUSTDOC_PREFIX}/{relative}", config, item)
    return site


def _ref() -> str:
    """Get the Git ref that links to files on GitHub use.

    Returns:
        The commit that Read the Docs builds, or `main` elsewhere.
    """
    return os.environ.get("READTHEDOCS_GIT_COMMIT_HASH", "main")


def on_nav(nav: Navigation, config: MkDocsConfig, files: Files) -> Navigation:
    """Point links of `nav` to files on GitHub at the commit that is built.

    Args:
        nav: Navigation of the site.
        config: MkDocs configuration (unused).
        files: Files of the site (unused).

    Returns:
        The navigation, with `blob/main/` links at the built commit.
    """
    prefix = f"{REPO_URL}/blob/main/"
    stack = list(nav.items)
    while stack:
        item = stack.pop()
        if isinstance(item, Link) and item.url.startswith(prefix):
            item.url = f"{REPO_URL}/blob/{_ref()}/{item.url.removeprefix(prefix)}"
        stack.extend(getattr(item, "children", None) or [])
    return nav


def _github_url(path: str, ref: str) -> str:
    """Build the GitHub URL of a file or directory of the repository.

    Args:
        path: Path relative to the repository root.
        ref: Git ref to link to.

    Returns:
        URL of the file (`blob`) or directory (`tree`).
    """
    kind = "tree" if (ROOT / path).is_dir() else "blob"
    return f"{REPO_URL}/{kind}/{ref}/{path}"


def _rewrite(target: str, page_path: str, site_paths: set[str], ref: str) -> str:
    """Point a link to a file of the repository outside the site to GitHub.

    Args:
        target: Link target, as written in the page.
        page_path: Path of the page in the repository.
        site_paths: Paths in the repository of the files of the site.
        ref: Git ref to link to.

    Returns:
        The GitHub URL, or `target` if it is not such a link.
    """
    bare = target[1:-1] if target.startswith("<") and target.endswith(">") else target
    parts = urlsplit(bare)
    if parts.scheme or parts.netloc or not parts.path or bare.startswith("/"):
        return target
    path = posixpath.normpath(posixpath.join(posixpath.dirname(page_path), parts.path))
    exists = (ROOT / path).exists() if TRACKED is None else path in TRACKED
    if path.startswith("../") or path in site_paths or not exists:
        return target
    fragment = f"#{parts.fragment}" if parts.fragment else ""
    return _github_url(path, ref) + fragment


def on_page_markdown(markdown: str, page: Any, config: MkDocsConfig, files: Files) -> str:
    """Point links to files outside the site to GitHub.

    Fenced code blocks and inline code spans are left as they are.

    Args:
        markdown: Markdown of the page.
        page: The page.
        config: MkDocs configuration (unused).
        files: Files of the site.

    Returns:
        Markdown with the links rewritten.
    """
    ref = _ref()
    site_paths = {file.src_uri for file in files}
    page_path = page.file.src_uri

    def replace(match: re.Match[str]) -> str:
        return match["head"] + _rewrite(match["target"], page_path, site_paths, ref)

    def rewrite_line(line: str) -> str:
        line = REFERENCE.sub(replace, line)
        parts = []
        start = 0
        for code in CODE_SPAN.finditer(line):
            parts.append(INLINE_LINK.sub(replace, line[start : code.start()]))
            parts.append(code[0])
            start = code.end()
        parts.append(INLINE_LINK.sub(replace, line[start:]))
        return "".join(parts)

    lines = []
    fence = ""
    for line in markdown.splitlines(keepends=True):
        opening = FENCE.match(line)
        if fence:
            if opening and opening[1].startswith(fence):
                fence = ""
        elif opening:
            fence = opening[1]
        else:
            line = rewrite_line(line)
        lines.append(line)
    return "".join(lines)
