"""The type stubs in `ga4gh_sdk.pyi` match the module."""

import ast
import difflib
import enum
import inspect
import re
import types
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



def documented_functions(node):
    """The methods of a stub class that carry their docstring.

    Of an overloaded method, that is the last declaration; the others end in `...`.
    """
    last = {}
    for item in node.body:
        if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
            last[item.name] = item
    return list(last.values())


def documented_attribute(body, index):
    """Whether a string literal follows the attribute or enum member at `body[index]`."""
    following = body[index + 1] if index + 1 < len(body) else None
    return (
        isinstance(following, ast.Expr)
        and isinstance(following.value, ast.Constant)
        and isinstance(following.value.value, str)
    )


def undocumented_members(node):
    """The names of a stub class's members without a docstring."""
    missing = [f.name for f in documented_functions(node) if ast.get_docstring(f) is None]
    for index, item in enumerate(node.body):
        if isinstance(item, (ast.AnnAssign, ast.Assign)) and not documented_attribute(
            node.body, index
        ):
            targets = [item.target] if isinstance(item, ast.AnnAssign) else item.targets
            missing.extend(t.id for t in targets if isinstance(t, ast.Name))
    return missing


ARGS_ENTRY = re.compile(r"^    (\*{0,2}\w+)(?: \(.*\))?:", re.MULTILINE)


def documented_parameters(function):
    """The parameter names in the `Args:` section of a function's docstring."""
    docstring = ast.get_docstring(function) or ""
    _, found, rest = docstring.partition("Args:\n")
    if not found:
        return []
    section = rest.split("\n\n", 1)[0]
    return ARGS_ENTRY.findall(section)


def signature_parameters(function):
    """A function's parameter names, without `self` and `cls`; `*args` and `**kwargs` starred."""
    args = function.args
    names = [arg.arg for arg in args.posonlyargs + args.args + args.kwonlyargs]
    if args.vararg:
        names.append(f"*{args.vararg.arg}")
    if args.kwarg:
        names.append(f"**{args.kwarg.arg}")
    return sorted(name for name in names if name not in ("self", "cls"))


def test_stub_documents_the_module():
    # The Python API reference (`api.md`) is generated from the stub's docstrings.
    assert ast.get_docstring(TREE)


@pytest.mark.parametrize("name", sorted(stub_class_nodes()))
def test_stub_documents_every_member(name):
    node = stub_class_nodes()[name]
    assert ast.get_docstring(node), name
    assert undocumented_members(node) == [], name


@pytest.mark.parametrize("name", sorted(stub_class_nodes()))
def test_stub_documents_every_parameter(name):
    # Griffe fails the docs build on documented parameters that the signature lacks;
    # this checks the reverse: every parameter is documented.
    for function in documented_functions(stub_class_nodes()[name]):
        documented = sorted(documented_parameters(function))
        assert documented == signature_parameters(function), f"{name}.{function.name}"


def runtime_docstrings(name):
    """Maps the class and each documented stub method of a class to its two docstrings.

    The values are `(stub, runtime)` pairs, keyed by `None` for the class and by method
    name otherwise. `help()` shows the runtime docstrings: PyO3 builds them from the Rust
    doc comments, and the classes defined in Python have them in their source. Left out
    are `__init__`, whose docstring PyO3 doesn't keep (`help()` shows the class's
    docstring), slots such as `__len__`, which have Python's own docstrings, and
    attributes and enum members, which carry no docstring of their own at runtime.
    """
    node = stub_class_nodes()[name]
    cls = getattr(ga4gh_sdk, name)
    pairs = {None: (ast.get_docstring(node), inspect.getdoc(cls))}
    for function in documented_functions(node):
        member = vars(cls).get(function.name)
        if function.name == "__init__" or isinstance(member, types.WrapperDescriptorType):
            continue
        doc = inspect.cleandoc(member.__doc__) if member.__doc__ else None
        pairs[function.name] = (ast.get_docstring(function), doc)
    return pairs


@pytest.mark.parametrize("name", sorted(stub_class_nodes()))
def test_runtime_docstrings_match_stub(name):
    # The Python API reference shows the stub's docstrings and `help()` the runtime ones;
    # copy a changed docstring to the Rust doc comment (or the Python source) word for word.
    for member, (stub, runtime) in runtime_docstrings(name).items():
        label = name if member is None else f"{name}.{member}"
        diff = difflib.unified_diff(
            (runtime or "").splitlines(), (stub or "").splitlines(), "runtime", "stub", lineterm=""
        )
        assert stub == runtime, f"{label}:\n" + "\n".join(diff)
