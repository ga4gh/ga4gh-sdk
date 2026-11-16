"""Griffe extension that renders stub functions declared only with `@overload`.

A stub has no implementation after its overloads, so Griffe keeps such a
function in `overloads` and not among the members, and mkdocstrings skips it.
The extension makes the last overload, which carries the docstring, the
member and lists the other overloads under it.
"""

from __future__ import annotations

import ast
from typing import Any

from griffe import Class, Extension, Inspector, Module, ObjectNode, Visitor


def promote_overloads(obj: Module | Class) -> None:
    """Make the last overload of each overload-only function a member of `obj`.

    Args:
        obj: The module or class whose overloads to promote.
    """
    for name, overloads in list(obj.overloads.items()):
        if not overloads or name in obj.members:
            continue
        function = overloads[-1]
        function.overloads = overloads[:-1]
        obj.set_member(name, function)
        del obj.overloads[name]


class OverloadOnly(Extension):
    """Promotes overload-only functions of modules and classes to members."""

    def on_module_members(
        self,
        *,
        node: ast.AST | ObjectNode,
        mod: Module,
        agent: Visitor | Inspector,
        **kwargs: Any,
    ) -> None:
        """Promotes the module's overload-only functions.

        Args:
            node: The module's node.
            mod: The module.
            agent: The visitor or inspector that loaded it.
            **kwargs: Further arguments, for compatibility.
        """
        promote_overloads(mod)

    def on_class_members(
        self,
        *,
        node: ast.AST | ObjectNode,
        cls: Class,
        agent: Visitor | Inspector,
        **kwargs: Any,
    ) -> None:
        """Promotes the class's overload-only methods.

        Args:
            node: The class's node.
            cls: The class.
            agent: The visitor or inspector that loaded it.
            **kwargs: Further arguments, for compatibility.
        """
        promote_overloads(cls)
