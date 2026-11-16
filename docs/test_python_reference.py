"""Tests that the Python API reference renders every member of the stubs."""

from __future__ import annotations

import ast
from pathlib import Path

import griffe
import pytest

import hooks

ROOT = Path(__file__).resolve().parent.parent
STUB = ROOT / "python" / "ga4gh_sdk.pyi"
EXTENSION = ROOT / "docs" / "griffe_overloads.py"


@pytest.fixture
def package(tmp_path: Path) -> griffe.Module:
    """Loads the stubs as the hooks stage them, with the overload extension.

    Args:
        tmp_path: The directory to stage the stubs in.

    Returns:
        The package `ga4gh_sdk`.
    """
    hooks.stage_stubs(tmp_path)
    return griffe.load(
        "ga4gh_sdk",
        search_paths=[tmp_path],
        extensions=griffe.load_extensions(str(EXTENSION)),
    )


def stub_members() -> set[str]:
    """Lists the public classes and their members as declared in the stub.

    Returns:
        Dotted names such as `Task.wait`; private names (`_x`) are left out,
        as the reference's filter leaves them out.
    """
    names: set[str] = set()
    for node in ast.parse(STUB.read_text(encoding="utf-8")).body:
        if not isinstance(node, ast.ClassDef) or node.name.startswith("_"):
            continue
        names.add(node.name)
        for item in node.body:
            if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                name = item.name
            elif isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name):
                name = item.target.id
            elif isinstance(item, ast.Assign) and isinstance(item.targets[0], ast.Name):
                name = item.targets[0].id
            else:
                continue
            if not (name.startswith("_") and not name.startswith("__")):
                names.add(f"{node.name}.{name}")
    return names


def loaded_members(package: griffe.Module) -> set[str]:
    """Lists the classes and members of `package`.

    Args:
        package: The package that Griffe loaded.

    Returns:
        Dotted names such as `Task.wait`, filtered as `stub_members()` filters.
    """
    names: set[str] = set()
    for cls in package.classes.values():
        if cls.name.startswith("_"):
            continue
        names.add(cls.name)
        for name in cls.members:
            if not (name.startswith("_") and not name.startswith("__")):
                names.add(f"{cls.name}.{name}")
    return names


def test_stage_stubs_copies_the_stubs(tmp_path: Path) -> None:
    """The staged `__init__.pyi` is a copy of the stubs."""
    staged = hooks.stage_stubs(tmp_path)
    assert staged == tmp_path / "ga4gh_sdk" / "__init__.pyi"
    assert staged.read_bytes() == STUB.read_bytes()


def test_reference_loads_every_stub_member(package: griffe.Module) -> None:
    """Griffe loads every class and member, overload-only methods included.

    Args:
        package: The staged package.
    """
    assert stub_members() - loaded_members(package) == set()


def test_overload_only_method_keeps_docstring_and_overloads(
    package: griffe.Module,
) -> None:
    """`Task.wait` is a member with its docstring and the other overloads.

    Args:
        package: The staged package.
    """
    wait = package["Task.wait"]
    assert wait.docstring is not None
    assert len(wait.overloads or []) == 2
