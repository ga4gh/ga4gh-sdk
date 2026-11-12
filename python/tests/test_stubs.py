"""The type stubs in `ga4gh_sdk.pyi` match the module."""

import ast
import enum
import inspect
from pathlib import Path

import pytest

import ga4gh_sdk

STUB = Path(__file__).parents[1] / "ga4gh_sdk.pyi"
TREE = ast.parse(STUB.read_text())


def stub_class_nodes():
    """Maps each class in the stub to its AST node."""
    return {node.name: node for node in TREE.body if isinstance(node, ast.ClassDef)}


def stub_aliases():
    """The names of module-level aliases of classes in the stub, such as `PyTesTask`."""
    classes = stub_class_nodes()
    return {
        target.id
        for node in TREE.body
        if isinstance(node, ast.Assign)
        and isinstance(node.value, ast.Name)
        and node.value.id in classes
        for target in node.targets
        if isinstance(target, ast.Name)
    }


def stub_members(node):
    """The names of the members that a stub class declares."""
    members = set()
    for item in node.body:
        if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
            members.add(item.name)
        elif isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name):
            members.add(item.target.id)
        elif isinstance(item, ast.Assign):
            members.update(t.id for t in item.targets if isinstance(t, ast.Name))
    return members


def stub_methods(node):
    """Maps each method of a stub class (not properties) to its parameter names."""
    methods = {}
    for item in node.body:
        if not isinstance(item, ast.FunctionDef):
            continue
        if any(isinstance(d, ast.Name) and d.id == "property" for d in item.decorator_list):
            continue
        args = item.args.posonlyargs + item.args.args + item.args.kwonlyargs
        methods[item.name] = [arg.arg for arg in args if arg.arg not in ("self", "cls")]
    return methods


def runtime_members(cls):
    """The public members that a class defines itself, plus `__init__` if it has a constructor.

    Dunder methods that the stub declares, such as `TesState.__str__`, count if the class
    defines them itself.
    """
    if issubclass(cls, enum.Enum):
        names = {state.name for state in cls}
    else:
        names = {name for name in vars(cls) if not name.startswith("_")}
    dunders = {name for name in stub_members(stub_class_nodes()[cls.__name__]) if name.startswith("__")}
    names |= {name for name in dunders - {"__init__"} if name in vars(cls)}
    # PyO3 sets `__text_signature__` on classes with a `#[new]` constructor.
    if getattr(cls, "__text_signature__", None) is not None:
        names.add("__init__")
    return names


def runtime_parameters(function):
    """The names of a function's named parameters, without `self`, `cls` and `*args`."""
    skipped = (inspect.Parameter.VAR_POSITIONAL, inspect.Parameter.VAR_KEYWORD)
    return [
        name
        for name, parameter in inspect.signature(function).parameters.items()
        if parameter.kind not in skipped and name not in ("self", "cls", "$cls")
    ]


def test_stub_declares_every_class():
    runtime = {name for name, value in vars(ga4gh_sdk).items() if inspect.isclass(value)}
    assert set(stub_class_nodes()) | stub_aliases() == runtime


@pytest.mark.parametrize("name", sorted(stub_class_nodes()))
def test_stub_declares_every_member(name):
    cls = getattr(ga4gh_sdk, name)
    assert stub_members(stub_class_nodes()[name]) == runtime_members(cls)


@pytest.mark.parametrize(
    "name", sorted(set(stub_class_nodes()) - {"TesState", "WesState", "ServiceType"})
)
def test_stub_parameters_match(name):
    cls = getattr(ga4gh_sdk, name)
    for method, parameters in stub_methods(stub_class_nodes()[name]).items():
        runtime = cls if method == "__init__" else getattr(cls, method)
        assert parameters == runtime_parameters(runtime), f"{name}.{method}"


def test_package_ships_stubs_and_marker():
    package = Path(ga4gh_sdk.__file__).parent
    assert (package / "py.typed").is_file()
    assert (package / "__init__.pyi").is_file()
