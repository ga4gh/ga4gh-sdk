"""Tests that the Python API reference renders every member of the stubs."""

from __future__ import annotations

import ast
from pathlib import Path

import griffe

ROOT = Path(__file__).resolve().parent.parent
STUB = ROOT / "python" / "ga4gh_sdk.pyi"
EXTENSION = ROOT / "docs" / "griffe_overloads.py"


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


def loaded_members() -> set[str]:
    """Lists the classes and members that Griffe loads with the extension.

    Returns:
        Dotted names such as `Task.wait`, filtered as `stub_members()` filters.
    """
    package = griffe.load(
        "ga4gh_sdk",
        search_paths=[ROOT / "python" / "stubs"],
        extensions=griffe.load_extensions(str(EXTENSION)),
    )
    names: set[str] = set()
    for cls in package.classes.values():
        if cls.name.startswith("_"):
            continue
        names.add(cls.name)
        for name in cls.members:
            if not (name.startswith("_") and not name.startswith("__")):
                names.add(f"{cls.name}.{name}")
    return names


def test_reference_loads_every_stub_member() -> None:
    """Griffe loads every class and member, overload-only methods included."""
    assert stub_members() - loaded_members() == set()


def test_overload_only_method_keeps_docstring_and_overloads() -> None:
    """`Task.wait` is a member with its docstring and the other overloads."""
    package = griffe.load(
        "ga4gh_sdk",
        search_paths=[ROOT / "python" / "stubs"],
        extensions=griffe.load_extensions(str(EXTENSION)),
    )
    wait = package["Task.wait"]
    assert wait.docstring is not None
    assert len(wait.overloads or []) == 2
