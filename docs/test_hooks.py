"""Tests of the MkDocs hooks (`docs/hooks.py`).

Run them with `python3 -m pytest docs/test_hooks.py`, with the `docs`
dependency group of `python/pyproject.toml` installed.
"""

from __future__ import annotations

from pathlib import Path
import sys
from types import SimpleNamespace

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import hooks  # noqa: E402

BLOB = f"{hooks.REPO_URL}/blob/main"


@pytest.fixture(autouse=True)
def _main_ref(monkeypatch: pytest.MonkeyPatch) -> None:
    """Link to `main`, as outside Read the Docs.

    Args:
        monkeypatch: pytest's monkeypatch fixture.
    """
    monkeypatch.delenv("READTHEDOCS_GIT_COMMIT_HASH", raising=False)


def _render(markdown: str, page: str = "README.md", site: tuple[str, ...] = ()) -> str:
    """Rewrite the links of a page.

    Args:
        markdown: Markdown of the page.
        page: Path of the page in the repository.
        site: Paths of the other files of the site.

    Returns:
        The Markdown that `on_page_markdown` returns.
    """
    files = [SimpleNamespace(src_uri=path) for path in (page, *site)]
    page_object = SimpleNamespace(file=SimpleNamespace(src_uri=page))
    return hooks.on_page_markdown(markdown, page_object, None, files)


def test_nav_paths() -> None:
    """Paths of nested sections and of entries without titles are listed."""
    nav = [{"Home": "README.md"}, {"Guides": [{"CLI": "cli/README.md"}, "docs/usage.md"]}]
    assert hooks._nav_paths(nav) == ["README.md", "cli/README.md", "docs/usage.md"]


def test_tracked_file_outside_site() -> None:
    """A link to a tracked file that is no page goes to GitHub."""
    assert _render("[x](LICENSE)") == f"[x]({BLOB}/LICENSE)"


def test_relative_to_page_with_fragment() -> None:
    """Links resolve relative to the page and keep their fragment."""
    assert _render("[x](../Makefile#L1)", page="docs/usage.md") == f"[x]({BLOB}/Makefile#L1)"


def test_rewritten_link_preserves_query_and_fragment() -> None:
    """A GitHub file link retains its query and fragment."""
    assert _render("[x](LICENSE?raw=1#L1)") == f"[x]({BLOB}/LICENSE?raw=1#L1)"


def test_directory() -> None:
    """A link to a directory goes to its tree."""
    assert _render("[x](lib/src/)") == f"[x]({hooks.REPO_URL}/tree/main/lib/src)"


def test_repository_root() -> None:
    """A link to the repository root goes to its tree."""
    assert _render("[x](../)", page="docs/usage.md") == f"[x]({hooks.REPO_URL}/tree/main)"


def test_escaped_backtick() -> None:
    """An escaped backtick doesn't start a code span."""
    assert _render("\\`a [x](LICENSE) `b`") == f"\\`a [x]({BLOB}/LICENSE) `b`"


def test_reference_definition() -> None:
    """Reference definitions are rewritten, footnotes are not."""
    assert _render("[x]: LICENSE \"t\"\n[^1]: LICENSE\n") == (
        f"[x]: {BLOB}/LICENSE \"t\"\n[^1]: LICENSE\n"
    )


@pytest.mark.parametrize(
    "link",
    [
        "[x](docs/usage.md)",
        "[x](https://example.org/LICENSE)",
        "[x](#anchor)",
        "[x](docs/missing.md)",
        "[x](target/doc/ga4gh_sdk/index.html)",
        "[x](../outside.md)",
    ],
)
def test_unchanged(link: str) -> None:
    """Pages, URLs, anchors and untracked or missing files stay.

    Args:
        link: The link.
    """
    assert _render(link, site=("docs/usage.md",)) == link


def test_code_is_unchanged() -> None:
    """Fenced code blocks and inline code spans stay."""
    markdown = "```md\n[x](LICENSE)\n```\n`[x](LICENSE)` [y](LICENSE)\n"
    assert _render(markdown) == (
        f"```md\n[x](LICENSE)\n```\n`[x](LICENSE)` [y]({BLOB}/LICENSE)\n"
    )


def test_read_the_docs_commit(monkeypatch: pytest.MonkeyPatch) -> None:
    """On Read the Docs, links go to the commit that is built.

    Args:
        monkeypatch: pytest's monkeypatch fixture.
    """
    monkeypatch.setenv("READTHEDOCS_GIT_COMMIT_HASH", "abc123")
    assert _render("[x](LICENSE)") == f"[x]({hooks.REPO_URL}/blob/abc123/LICENSE)"
