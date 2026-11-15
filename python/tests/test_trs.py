"""The TRS bindings: version selection, tools, versions, descriptors, files and pages."""

import json
import os
import subprocess
import sys
import threading
from pathlib import Path
from urllib.parse import quote

import pytest

from ga4gh_sdk import (
    TRS,
    Configuration,
    FileWrapper,
    ServiceType,
    Tool,
    ToolFile,
    ToolPage,
    ToolVersion,
    Transport,
)

from .mock_server import (
    TRS_DESCRIPTOR,
    TRS_FILES,
    TRS_PLAIN_DESCRIPTOR,
    TRS_TOOL,
    TRS_TOOL_ID,
    TRS_VERSION,
    TRS_VERSION_ID,
    TRS_VERSION_PATH,
    TRS_ZIP,
)

TOOL_PATH = f"/trs/tools/{quote(TRS_TOOL_ID, safe='')}"
VERSION_PATH = f"/trs/{TRS_VERSION_PATH}"


@pytest.fixture
def trs_config(server):
    return Configuration(f"{server.root}/trs")


@pytest.fixture
def trs(trs_config):
    return TRS(trs_config)


# Connecting and version selection


def test_client_reads_service_info_and_selects_version(server, trs):
    assert server.last()["path"] == "/trs/service-info"
    assert (trs.api_version, trs.declared_version, trs.version_source) == ("2.0.1", "2.0.1", "declared")
    assert trs.tag_filter == "not_specified"
    assert trs.discovered_service_info["type"] == {"group": "org.ga4gh", "artifact": "trs", "version": "2.0.1"}
    assert trs.config.get_base_path() == f"{server.root}/trs"
    assert isinstance(trs.transport, Transport)


def test_client_selects_trs_2_0_2(server):
    trs = TRS(Configuration(f"{server.root}/trs2"))
    assert (trs.api_version, trs.version_source, trs.tag_filter) == ("2.0.2", "declared", "specified")


def test_unknown_version_reads_as_2_0_1_baseline(server):
    trs = TRS(Configuration(f"{server.root}/trs-next"))
    assert (trs.api_version, trs.declared_version, trs.version_source) == ("2.0.1", "9.9.9", "baseline")
    assert trs.tag_filter == "unknown"


def test_strict_version_rejects_unknown_version(server):
    with pytest.raises(RuntimeError, match="not recognized"):
        TRS(Configuration(f"{server.root}/trs-next"), api_version="strict")
    assert TRS(Configuration(f"{server.root}/trs2"), api_version="strict").api_version == "2.0.2"


def test_explicit_version_keeps_declared_version(server):
    trs = TRS(Configuration(f"{server.root}/trs"), api_version="2.0.2", tag_filter="disabled")
    assert (trs.api_version, trs.declared_version, trs.version_source) == ("2.0.2", "2.0.1", "caller")
    assert trs.tag_filter == "caller_disabled"


def test_client_without_discovery_sends_no_request(server):
    before = server.paths()
    trs = TRS(Configuration(f"{server.root}/trs"), api_version="2.0.2", discovery=False)
    assert server.paths() == before
    assert (trs.api_version, trs.declared_version, trs.version_source) == ("2.0.2", None, "caller")
    assert trs.discovered_service_info is None
    assert trs.get_tool(TRS_TOOL_ID).id == TRS_TOOL_ID


@pytest.mark.parametrize(
    "kwargs, message",
    [
        ({"discovery": False}, "explicit api_version"),
        ({"api_version": "strict", "discovery": False}, "explicit api_version"),
        ({"api_version": "2.1"}, "Invalid api_version"),
        ({"tag_filter": "on"}, "Invalid tag_filter"),
    ],
)
def test_invalid_options_raise_value_error(server, trs_config, kwargs, message):
    before = server.paths()
    with pytest.raises(ValueError, match=message):
        TRS(trs_config, **kwargs)
    assert server.paths() == before


def test_client_checks_service_type(server, config):
    with pytest.raises(RuntimeError, match="not an instance of TRS"):
        TRS(config)


def test_client_reports_unreachable_service():
    with pytest.raises(RuntimeError, match="Failed to create TRS instance"):
        TRS(Configuration("http://127.0.0.1:1/trs"))


def test_configured_credentials_reach_every_trs_request(server):
    config = Configuration(f"{server.root}/trs")
    config.set_bearer_access_token("test-bearer")
    trs = TRS(config)

    def authorization():
        headers = {name.lower(): value for name, value in server.last()["headers"].items()}
        return headers.get("authorization")

    assert (server.last()["path"], authorization()) == ("/trs/service-info", "Bearer test-bearer")
    page = trs.list_tools(limit=1)
    assert authorization() == "Bearer test-bearer"
    trs.next_tools_page(page.next_page)
    assert (server.last()["query"], authorization()) == ({"offset": ["1"], "limit": ["1"]}, "Bearer test-bearer")
    trs.get_descriptor(TRS_TOOL_ID, TRS_VERSION_ID, "PLAIN_CWL")
    assert authorization() == "Bearer test-bearer"
    trs.get_files_zip(TRS_TOOL_ID, TRS_VERSION_ID, "CWL")
    assert authorization() == "Bearer test-bearer"


def test_service_info_is_read_again(server, trs):
    info = trs.service_info()
    assert server.last()["path"] == "/trs/service-info"
    assert info["id"] == "org.example.trs"


# Tools and versions


def test_get_tool_keeps_identifiers_and_unknown_fields(server, trs):
    tool = trs.get_tool(TRS_TOOL_ID)
    request = server.last()
    assert request["path"] == TOOL_PATH
    headers = {name.lower(): value for name, value in request["headers"].items()}
    assert headers["accept"] == "application/json"
    assert isinstance(tool, Tool)
    assert (tool.id, tool.url, tool.name, tool.organization) == (
        TRS_TOOL_ID,
        TRS_TOOL["url"],
        "repo",
        "org",
    )
    assert tool.aliases == ["repo"]
    assert tool.toolclass == TRS_TOOL["toolclass"]
    assert (tool.description, tool.has_checker, tool.checker_url, tool.meta_version) == (
        "A workflow",
        False,
        None,
        None,
    )
    assert tool.to_dict() == TRS_TOOL
    assert json.loads(tool.json()) == TRS_TOOL
    assert repr(tool) == f"Tool(id={TRS_TOOL_ID!r}, name='repo')"
    [version] = tool.versions
    assert isinstance(version, ToolVersion)
    assert version.to_dict() == TRS_VERSION


def test_version_fields(server, trs):
    version = trs.get_version(TRS_TOOL_ID, TRS_VERSION_ID)
    assert server.last()["path"] == VERSION_PATH
    assert (version.id, version.url, version.name, version.author) == (
        TRS_VERSION_ID,
        TRS_VERSION["url"],
        "First release",
        ["Ada"],
    )
    assert version.description == "The first release"
    assert version.descriptor_type == ["CWL"]
    assert version.descriptor_type_version == {"CWL": ["v1.2"]}
    assert version.images == TRS_VERSION["images"]
    assert (version.is_production, version.verified, version.signed, version.containerfile) == (
        True,
        False,
        None,
        None,
    )
    assert (version.meta_version, version.verified_source, version.included_apps) == (None, None, None)
    assert version.to_dict() == TRS_VERSION
    assert repr(version) == f"ToolVersion(id={TRS_VERSION_ID!r}, name='First release')"


def test_list_versions(server, trs):
    versions = trs.list_versions(TRS_TOOL_ID)
    assert server.last()["path"] == f"{TOOL_PATH}/versions"
    assert [version.to_dict() for version in versions] == [TRS_VERSION]


def test_list_tool_classes(server, trs):
    assert trs.list_tool_classes() == [TRS_TOOL["toolclass"]]
    assert server.last()["path"] == "/trs/toolClasses"


@pytest.mark.parametrize("root", ["trs", "trs2"])
def test_non_string_version_description_is_kept_in_json(server, root):
    # TRS 2.0.2 defines `description` as a string; other values stay in the JSON.
    [version] = TRS(Configuration(f"{server.root}/{root}")).get_tool("bad-description").versions
    assert version.description is None
    assert version.to_dict()["description"] == 7


# Listing tools and pages


def test_list_tools_sends_filters(server, trs):
    page = trs.list_tools(
        tool_class="Workflow",
        descriptor_type="CWL",
        organization="org",
        name="tool",
        toolname="repo",
        checker=False,
        offset="0",
        limit=1,
    )
    request = server.last()
    assert request["path"] == "/trs/tools"
    assert request["query"] == {
        "toolClass": ["Workflow"],
        "descriptorType": ["CWL"],
        "organization": ["org"],
        "name": ["tool"],
        "toolname": ["repo"],
        "checker": ["false"],
        "offset": ["0"],
        "limit": ["1"],
    }
    assert isinstance(page, ToolPage)
    assert [tool.to_dict() for tool in page.tools] == [TRS_TOOL]
    assert len(page) == 1


def test_list_tools_without_filters_sends_no_query(server, trs):
    trs.list_tools()
    assert server.last()["raw_query"] == ""


@pytest.mark.parametrize("root", ["trs", "trs2"])
def test_pages_follow_next_page_below_each_root(server, root):
    trs = TRS(Configuration(f"{server.root}/{root}"))
    first = trs.list_tools(limit=1)
    assert first.next_page == f"{server.root}/{root}/tools?offset=1&limit=1"
    second = trs.next_tools_page(first.next_page)
    request = server.last()
    assert (request["path"], request["query"]) == (f"/{root}/tools", {"offset": ["1"], "limit": ["1"]})
    assert [tool.id for tool in second.tools] == ["second"]


def test_paging_headers_use_first_text_values(server):
    page = TRS(Configuration(f"{server.root}/trs/odd-headers"), api_version="2.0.1", discovery=False).list_tools()
    assert page.next_page == f"{server.root}/trs/tools?offset=1"
    assert page.last_page is None
    assert page.self_link is None
    headers = page.headers()
    assert headers["next_page"] == f"{server.root}/trs/tools?offset=1, {server.root}/trs/tools?offset=2"
    assert headers["last_page"] == ""
    assert "self_link" not in headers


def test_pages_follow_next_page(server, trs):
    first = trs.list_tools(limit=1)
    assert first.next_page == f"{server.root}/trs/tools?offset=1&limit=1"
    assert first.last_page == first.next_page
    assert (first.current_offset, first.current_limit) == ("0", "1")
    assert first.self_link is None
    assert first.headers()["current_limit"] == "1"
    assert repr(first) == "ToolPage(tools=1, next_page=True)"

    second = trs.next_tools_page(first.next_page)
    request = server.last()
    assert request["path"] == "/trs/tools"
    assert request["query"] == {"offset": ["1"], "limit": ["1"]}
    assert [tool.id for tool in second.tools] == ["second"]
    assert second.next_page is None
    assert repr(second) == "ToolPage(tools=1, next_page=False)"


@pytest.mark.parametrize(
    "next_page",
    ["https://other.example/trs/tools?offset=1", "http://127.0.0.1:1/elsewhere/tools"],
)
def test_next_page_must_stay_at_the_api_root(server, trs, next_page):
    before = server.paths()
    with pytest.raises(RuntimeError, match="same API root"):
        trs.next_tools_page(next_page)
    assert server.paths() == before


def test_empty_next_page_raises_value_error(trs):
    with pytest.raises(ValueError, match="next_page"):
        trs.next_tools_page("")


def test_tags_need_trs_2_0_2_or_an_override(server, trs, trs_config):
    before = server.paths()
    with pytest.raises(ValueError, match="'tags' is not established"):
        trs.list_tools(tags=["genomics"])
    assert server.paths() == before

    TRS(trs_config, tag_filter="enabled").list_tools(tags=["genomics", "rna"])
    assert server.last()["raw_query"] == "tags=genomics%2Crna"

    TRS(Configuration(f"{server.root}/trs2")).list_tools(tags=["genomics"])
    assert server.last()["query"] == {"tags": ["genomics"]}


def test_tags_follow_the_tag_filter_override(server):
    trs2 = TRS(Configuration(f"{server.root}/trs2"), tag_filter="disabled")
    assert trs2.tag_filter == "caller_disabled"
    before = server.paths()
    with pytest.raises(ValueError, match="'tags' is not established"):
        trs2.list_tools(tags=["genomics"])
    assert server.paths() == before

    trs = TRS(Configuration(f"{server.root}/trs"), api_version="2.0.1", tag_filter="enabled", discovery=False)
    assert trs.tag_filter == "caller_enabled"
    trs.list_tools(tags=["genomics"])
    assert server.last()["query"] == {"tags": ["genomics"]}


@pytest.mark.parametrize("limit", [0, -1, 2**31, 2**63, 2**100])
def test_invalid_page_size_raises_value_error(server, trs, limit):
    before = server.paths()
    with pytest.raises(ValueError, match="limit must be between"):
        trs.list_tools(limit=limit)
    assert server.paths() == before


def test_unknown_descriptor_type_filter_is_sent_as_given(server, trs):
    trs.list_tools(descriptor_type="JUPYTER")
    assert server.last()["query"] == {"descriptorType": ["JUPYTER"]}


@pytest.mark.parametrize("tags", [["a,b"], [""]])
def test_tags_that_cannot_be_sent_raise_value_error(server, tags):
    trs = TRS(Configuration(f"{server.root}/trs2"))
    before = server.paths()
    with pytest.raises(ValueError, match="comma"):
        trs.list_tools(tags=tags)
    assert server.paths() == before


def test_filters_are_keyword_only(trs):
    with pytest.raises(TypeError):
        trs.list_tools("Workflow")


# Descriptors and files


def test_get_descriptor(server, trs):
    descriptor = trs.get_descriptor(TRS_TOOL_ID, TRS_VERSION_ID, "CWL")
    assert server.last()["path"] == f"{VERSION_PATH}/CWL/descriptor"
    assert isinstance(descriptor, FileWrapper)
    assert (descriptor.content, descriptor.url, descriptor.image_type) == (
        TRS_DESCRIPTOR["content"],
        TRS_DESCRIPTOR["url"],
        "CWL",
    )
    assert descriptor.checksum == TRS_DESCRIPTOR["checksum"]
    assert descriptor.to_dict() == TRS_DESCRIPTOR
    assert json.loads(descriptor.json()) == TRS_DESCRIPTOR
    assert repr(descriptor) == f"FileWrapper(url={TRS_DESCRIPTOR['url']!r}, content=<33 characters>)"


def test_get_plain_descriptor(server, trs):
    descriptor = trs.get_descriptor(TRS_TOOL_ID, TRS_VERSION_ID, "PLAIN_CWL")
    assert server.last()["path"] == f"{VERSION_PATH}/PLAIN_CWL/descriptor"
    assert descriptor.content == TRS_PLAIN_DESCRIPTOR
    assert descriptor.to_dict() == {"content": TRS_PLAIN_DESCRIPTOR}
    assert descriptor.url is None


def test_get_descriptor_at_relative_path(server, trs):
    descriptor = trs.get_descriptor(TRS_TOOL_ID, TRS_VERSION_ID, "CWL", relative_path="tools/align.cwl")
    assert server.last()["path"] == f"{VERSION_PATH}/CWL/descriptor/tools%2Falign.cwl"
    assert descriptor.content == "class: CommandLineTool\n"


def test_list_tests(server, trs):
    [test] = trs.list_tests(TRS_TOOL_ID, TRS_VERSION_ID, "CWL")
    assert test.to_dict() == TRS_DESCRIPTOR
    [plain] = trs.list_tests(TRS_TOOL_ID, TRS_VERSION_ID, "PLAIN_CWL")
    assert server.last()["path"] == f"{VERSION_PATH}/PLAIN_CWL/tests"
    assert plain.content == "message: hello\n"


def test_list_files(server, trs):
    files = trs.list_files(TRS_TOOL_ID, TRS_VERSION_ID, "CWL")
    assert server.last()["path"] == f"{VERSION_PATH}/CWL/files"
    assert all(isinstance(file, ToolFile) for file in files)
    assert [(file.path, file.file_type) for file in files] == [
        ("main.cwl", "PRIMARY_DESCRIPTOR"),
        ("tools/align.cwl", "SECONDARY_DESCRIPTOR"),
        ("notes.md", "NOT_A_TRS_TYPE"),
    ]
    assert files[0].checksum is None
    assert [file.to_dict() for file in files] == TRS_FILES
    assert repr(files[0]) == "ToolFile(path='main.cwl', file_type='PRIMARY_DESCRIPTOR')"


def test_get_files_zip(server, trs):
    archive = trs.get_files_zip(TRS_TOOL_ID, TRS_VERSION_ID, "CWL")
    request = server.last()
    assert (request["path"], request["query"]) == (f"{VERSION_PATH}/CWL/files", {"format": ["zip"]})
    assert archive == TRS_ZIP
    assert isinstance(archive, bytes)


def test_get_files_zip_checks_limit_and_signature(server, trs):
    with pytest.raises(RuntimeError, match="Failed to get files as ZIP: .*exceeds body limit of 4 bytes"):
        trs.get_files_zip(TRS_TOOL_ID, TRS_VERSION_ID, "CWL", limit=4)
    # A registry that ignores `format=zip` and sends the file list.
    with pytest.raises(RuntimeError, match="not a ZIP archive"):
        trs.get_files_zip("t", "1", "CWL")
    before = server.paths()
    for limit in (0, -1, 2**63, 2**100):
        with pytest.raises(ValueError, match="limit must be positive"):
            trs.get_files_zip(TRS_TOOL_ID, TRS_VERSION_ID, "CWL", limit=limit)
    assert server.paths() == before


def test_get_containerfiles(server, trs):
    [file] = trs.get_containerfiles(TRS_TOOL_ID, TRS_VERSION_ID)
    assert server.last()["path"] == f"{VERSION_PATH}/containerfile"
    assert (file.content, file.image_type) == ("FROM alpine\n", "Docker")


# Errors


@pytest.mark.parametrize(
    "call",
    [
        lambda trs: trs.get_tool(""),
        lambda trs: trs.get_tool(".."),
        lambda trs: trs.list_versions("."),
        lambda trs: trs.get_version(TRS_TOOL_ID, ""),
        lambda trs: trs.get_descriptor(TRS_TOOL_ID, "..", "CWL"),
        lambda trs: trs.get_descriptor(TRS_TOOL_ID, TRS_VERSION_ID, ""),
        lambda trs: trs.get_descriptor(TRS_TOOL_ID, TRS_VERSION_ID, "CWL", relative_path=".."),
        lambda trs: trs.list_tests("", TRS_VERSION_ID, "CWL"),
        lambda trs: trs.list_files(TRS_TOOL_ID, TRS_VERSION_ID, "."),
        lambda trs: trs.get_files_zip("..", TRS_VERSION_ID, "CWL"),
        lambda trs: trs.get_containerfiles(TRS_TOOL_ID, ""),
        lambda trs: trs.list_tools(descriptor_type=""),
    ],
)
def test_invalid_ids_raise_value_error_before_a_request(server, trs, call):
    before = server.paths()
    with pytest.raises(ValueError, match="must not be"):
        call(trs)
    assert server.paths() == before


def test_missing_tool_raises_runtime_error_with_status(server, trs):
    with pytest.raises(RuntimeError, match="Failed to get tool: .*404"):
        trs.get_tool("missing")
    assert server.last()["path"] == "/trs/tools/missing"


def test_response_that_does_not_fit_raises_runtime_error(server, trs):
    with pytest.raises(RuntimeError, match="Failed to get tool: .*missing field"):
        trs.get_tool("bad")
    assert server.last()["path"] == "/trs/tools/bad"


def test_client_can_be_used_from_several_threads(trs):
    results = []

    def get():
        results.append(trs.get_tool(TRS_TOOL_ID).id)

    threads = [threading.Thread(target=get) for _ in range(4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=10)
    assert results == [TRS_TOOL_ID] * 4


# Configuration, subclasses and the example


@pytest.mark.skipif(os.name != "posix", reason="HOME controls the config path on Unix")
def test_trs_section_loads_into_client(server, tmp_path, monkeypatch):
    config_dir = tmp_path / ".ga4gh-cli"
    config_dir.mkdir()
    (config_dir / "config.json").write_text(json.dumps({"TRS": {"base_path": f"{server.root}/trs2"}}))
    monkeypatch.setenv("HOME", str(tmp_path))
    config = Configuration("https://example.org")
    config.from_file(ServiceType.TRS)
    assert TRS(config).api_version == "2.0.2"


@pytest.mark.parametrize("cls", [TRS, Tool, ToolVersion, FileWrapper, ToolFile, ToolPage])
def test_classes_can_be_subclassed(cls):
    subclass = type(f"My{cls.__name__}", (cls,), {})
    assert issubclass(subclass, cls)


def test_subclass_init_can_take_further_arguments(trs_config):
    class Extended(TRS):
        def __init__(self, *args, token, **kwargs):
            super().__init__()  # __new__ has already set up the base class
            self.token = token

    instance = Extended(trs_config, "extra", token="t", api_version="2.0.2")
    assert isinstance(instance, TRS)
    assert (instance.token, instance.api_version) == ("t", "2.0.2")


def test_unexpected_arguments_raise_type_error(trs_config):
    plain_subclass = type("PlainTRS", (TRS,), {})
    for target in (TRS, plain_subclass):
        with pytest.raises(TypeError, match="unexpected"):
            target(trs_config, "extra")


def test_trs_subclass_calls_extra_endpoint(server, trs_config):
    class ClassesTRS(TRS):
        def class_names(self):
            return [item["name"] for item in json.loads(self.transport.get("toolClasses"))]

    assert ClassesTRS(trs_config).class_names() == ["Workflow"]
    assert server.last()["path"] == "/trs/toolClasses"


def test_checked_in_example_runs_against_mock(server):
    result = subprocess.run(
        [
            sys.executable,
            "examples/trs_read.py",
            f"{server.root}/trs",
            TRS_TOOL_ID,
            TRS_VERSION_ID,
            "CWL",
        ],
        cwd=Path(__file__).resolve().parents[1],
        text=True,
        capture_output=True,
        timeout=20,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [
        "TRS version: 2.0.1",
        f"Version: {TRS_VERSION_ID} First release",
        f"Descriptor: 33 characters {TRS_DESCRIPTOR['url']}",
        "File: PRIMARY_DESCRIPTOR main.cwl",
        "File: SECONDARY_DESCRIPTOR tools/align.cwl",
        "File: NOT_A_TRS_TYPE notes.md",
    ]
