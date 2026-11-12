"""Reading built-in and custom sections of the configuration file."""

import json
import os

import pytest

from GA4GH import Configuration, ServiceType

pytestmark = pytest.mark.skipif(
    os.name != "posix", reason="HOME controls the config path on Unix"
)


@pytest.fixture
def write_config(tmp_path, monkeypatch):
    """Writes `~/.ga4gh-cli/config.json` below a temporary HOME."""

    def write(document):
        config_dir = tmp_path / ".ga4gh-cli"
        config_dir.mkdir(exist_ok=True)
        (config_dir / "config.json").write_text(json.dumps(document))

    monkeypatch.setenv("HOME", str(tmp_path))
    return write


def test_built_in_sections_have_empty_extra(write_config):
    write_config(
        {
            "TES": {
                "base_path": "https://example.org/ga4gh/tes/v1",
                "basic_auth": {"username": "alice", "password": "change-me"},
                "oauth_access_token": "my-token",
            },
            "WES": {"base_path": "https://example.org/ga4gh/wes/v1"},
            "REGISTRY": {"base_path": "https://example.org/ga4gh/registry/v1"},
        }
    )
    config = Configuration("https://wrong.example")
    config.from_file(ServiceType.TES)
    assert config.get_base_path() == "https://example.org/ga4gh/tes/v1"
    assert config.get_extra() == {}

    config.from_file(ServiceType.WES)
    assert config.get_base_path() == "https://example.org/ga4gh/wes/v1"

    config.from_file(ServiceType.REGISTRY)
    assert config.get_base_path() == "https://example.org/ga4gh/registry/v1"


def test_custom_section_keeps_extra_keys(write_config):
    write_config(
        {
            "TES": {"base_path": "https://example.org/ga4gh/tes/v1"},
            "PROXY": {
                "base_path": "https://example.org/proxy/v1",
                "oauth_access_token": "token",
                "timeout_seconds": 30,
                "routing": {"region": "eu"},
            },
        }
    )
    config = Configuration("https://wrong.example")
    config.from_file("PROXY")
    assert config.get_base_path() == "https://example.org/proxy/v1"
    assert config.get_extra() == {"timeout_seconds": 30, "routing": {"region": "eu"}}


def test_token_from_named_environment_variable(write_config, monkeypatch):
    write_config(
        {
            "PROXY": {
                "base_path": "https://example.org/proxy/v1",
                "oauth_access_token_env": "GA4GH_SDK_TEST_PROXY_TOKEN",
            }
        }
    )
    config = Configuration("https://wrong.example")
    monkeypatch.delenv("GA4GH_SDK_TEST_PROXY_TOKEN", raising=False)
    with pytest.raises(ValueError, match="'GA4GH_SDK_TEST_PROXY_TOKEN' is not set"):
        config.from_file("PROXY")

    monkeypatch.setenv("GA4GH_SDK_TEST_PROXY_TOKEN", "token")
    config.from_file("PROXY")
    assert config.get_base_path() == "https://example.org/proxy/v1"
    assert config.get_extra() == {}


def test_profiles_file_reads_the_default_profile(write_config):
    write_config(
        {
            "default_profile": "remote",
            "profiles": {
                "default": {"TES": {"base_path": "https://default.example.org/"}},
                "remote": {"TES": {"base_path": "https://remote.example.org/"}},
            },
        }
    )
    config = Configuration("https://wrong.example")
    config.from_file(ServiceType.TES)
    assert config.get_base_path() == "https://remote.example.org/"


def test_missing_custom_section_is_value_error(write_config):
    write_config({"TES": {"base_path": "https://example.org/ga4gh/tes/v1"}})
    config = Configuration("https://wrong.example")
    with pytest.raises(ValueError, match="'PROXY' object"):
        config.from_file("PROXY")


def test_section_must_be_service_type_or_str(write_config):
    write_config({"TES": {"base_path": "https://example.org/ga4gh/tes/v1"}})
    config = Configuration("https://wrong.example")
    with pytest.raises(TypeError, match="must be a ServiceType or a str, not int"):
        config.from_file(1)
