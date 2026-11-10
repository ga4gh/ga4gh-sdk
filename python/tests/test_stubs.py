"""The type stubs in `GA4GH.pyi` match the module."""

import ast
import inspect
from pathlib import Path

import pytest

import GA4GH

STUB = Path(__file__).parents[1] / "GA4GH.pyi"


def stub_classes():
    """Maps each class in the stub to the names of its members."""
    tree = ast.parse(STUB.read_text())
    classes = {}
    for node in tree.body:
        if isinstance(node, ast.ClassDef):
            members = set()
            for item in node.body:
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    members.add(item.name)
                elif isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name):
                    members.add(item.target.id)
                elif isinstance(item, ast.Assign):
                    members.update(t.id for t in item.targets if isinstance(t, ast.Name))
            classes[node.name] = members
    return classes


def runtime_members(cls):
    """The public members that a class defines itself, plus `__init__` if it has a constructor."""
    if issubclass(cls, GA4GH.TesState):
        return {state.name for state in cls} | {"__str__", "__format__"}
    names = {name for name in vars(cls) if not name.startswith("_")}
    # PyO3 sets `__text_signature__` on classes with a `#[new]` constructor.
    return names | ({"__init__"} if cls.__text_signature__ is not None else set())


def test_stub_declares_every_class():
    runtime = {name for name, value in vars(GA4GH).items() if inspect.isclass(value)}
    assert set(stub_classes()) | {"PyTesTask"} == runtime


@pytest.mark.parametrize("name", sorted(stub_classes()))
def test_stub_declares_every_member(name):
    cls = getattr(GA4GH, name)
    assert stub_classes()[name] == runtime_members(cls)
