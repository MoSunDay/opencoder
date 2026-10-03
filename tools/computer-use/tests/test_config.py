import json

import pytest

from opencoder_computer.config import endpoint_value, load_settings, parse_settings
from opencoder_computer.locks import exclusive_lock, target_lock_path


def document(os_type="linux"):
    return {
        "model": {"name": "anthropic/claude-sonnet-4-5", "api_key_file": "model.key"},
        "targets": {
            "work": {
                "os": os_type,
                "url": "https://desktop.example/cua/",
                "headers_file": "headers.json",
            }
        },
    }


@pytest.mark.parametrize("os_type", ["windows", "macos", "linux"])
def test_config_reads_credentials_relative_to_config(tmp_path, os_type):
    config = tmp_path / "config.json"
    config.write_text(json.dumps(document(os_type)))
    (tmp_path / "model.key").write_text("model-secret\n")
    (tmp_path / "headers.json").write_text('{"Authorization":"Bearer desktop-secret"}')
    settings = load_settings(config, "work")
    assert settings.os_type == os_type
    assert settings.endpoint == "https://desktop.example/cua"
    assert settings.api_key == "model-secret"
    assert settings.headers == {"Authorization": "Bearer desktop-secret"}
    assert "secret" not in repr(settings)


@pytest.mark.parametrize(
    "url",
    [
        "ws://host",
        "http://u:p@host",
        "https://host?q=key",
        "https://host#fragment",
        "http://host:invalid",
        "",
    ],
)
def test_config_rejects_invalid_or_secret_urls(url):
    with pytest.raises(ValueError):
        endpoint_value(url, "target.url")


def test_config_rejects_missing_target_and_invalid_headers():
    with pytest.raises(ValueError, match="unknown target"):
        parse_settings(document(), "missing", lambda _: "")
    with pytest.raises(ValueError, match="headers_file"):
        parse_settings(
            document(),
            "work",
            lambda name: (
                '{"Authorization":"invalid\\nheader"}' if name == "headers.json" else "key"
            ),
        )
    with pytest.raises(ValueError, match="target.os"):
        parse_settings(document("wayland"), "work", lambda _: "")


def test_target_aliases_share_a_lock(tmp_path):
    first = target_lock_path("http://localhost:8000/", tmp_path)
    assert first == target_lock_path("http://127.0.0.1:8000", tmp_path)
    with exclusive_lock(first):
        with pytest.raises(RuntimeError, match="already in use"):
            with exclusive_lock(first):
                pytest.fail("second owner entered")
    with exclusive_lock(first):
        pass


def test_lock_does_not_hide_body_errors(tmp_path):
    with pytest.raises(PermissionError, match="body error"):
        with exclusive_lock(tmp_path / "lock"):
            raise PermissionError("body error")
