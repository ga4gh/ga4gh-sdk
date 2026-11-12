"""Opt-in smoke tests of the TRS bindings against public registries.

They need network access, so they run only with ``GA4GH_TRS_PUBLIC_TESTS=1``::

    GA4GH_TRS_PUBLIC_TESTS=1 python -m pytest tests/test_trs_public.py
"""

import os

import pytest

from ga4gh_sdk import TRS, Configuration

pytestmark = pytest.mark.skipif(
    os.environ.get("GA4GH_TRS_PUBLIC_TESTS") != "1",
    reason="set GA4GH_TRS_PUBLIC_TESTS=1 to call public TRS registries",
)


def first_version_with_descriptor_type(tools):
    """A tool, one of its versions and that version's first descriptor type."""
    for tool in tools:
        for version in tool.versions:
            if version.descriptor_type:
                return tool, version, version.descriptor_type[0]
    pytest.fail("no tool version with a descriptor type")


def test_workflowhub_lists_tools_and_fetches_a_descriptor():
    trs = TRS(Configuration("https://workflowhub.eu/ga4gh/trs/v2"))
    page = trs.list_tools(tool_class="Workflow", limit=5)
    assert page.tools
    tool, version, descriptor_type = first_version_with_descriptor_type(page.tools)
    descriptor = trs.get_descriptor(tool.id, version.id, f"PLAIN_{descriptor_type}")
    assert descriptor.content
    files = trs.list_files(tool.id, version.id, descriptor_type)
    assert any(file.file_type == "PRIMARY_DESCRIPTOR" for file in files)


def test_dockstore_fetches_a_workflow_by_encoded_id():
    trs = TRS(Configuration("https://dockstore.org/api/ga4gh/trs/v2/"))
    tool_id = "#workflow/github.com/denis-yuen/dockstore-whalesay"
    assert trs.get_tool(tool_id).id == tool_id
    # Dockstore's version ID path parameter is the version's name; its
    # `ToolVersion.id` is `<tool ID>:<name>`.
    name = trs.list_versions(tool_id)[0].name
    descriptor = trs.get_descriptor(tool_id, name, "PLAIN_CWL")
    assert "class: Workflow" in descriptor.content
    assert trs.list_files(tool_id, name, "CWL")
