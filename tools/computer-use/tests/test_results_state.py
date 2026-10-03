import io
import json
import os

import pytest

from opencoder_computer.config import Settings
from opencoder_computer.locks import exclusive_lock
from opencoder_computer.results import diagnostics, redact, response_error, terminal_result
from opencoder_computer.state import RunFiles, read_status, request_cancel


def test_redaction_removes_secrets_and_inline_images():
    result = redact(
        {
            "authorization": "secret",
            "summary": "key-secret",
            "output": {"image_url": "data:image/png;base64,AAAA"},
        },
        ("key-secret",),
    )
    assert result == {"authorization": "[redacted]", "summary": "[redacted]", "output": {}}
    assert "AAAA" not in redact("image=data:image/png;base64,AAAA")


def test_diagnostics_preserve_json_stdout_and_redact_split_secrets(capsys):
    stream = io.StringIO()
    with diagnostics(stream, ("model-secret",)):
        print("model-sec", end="")
        print("ret")
        print("tail", end="")
    assert stream.getvalue() == "[redacted]\ntail"
    assert capsys.readouterr().out == ""


@pytest.mark.parametrize(
    ("header", "scheme", "token"),
    [
        ("aUtHoRiZaTiOn", "Bearer", "desktop-secret"),
        ("Proxy-Authorization", "Basic", "ZmFrZTpmYWtl"),
    ],
)
def test_authentication_errors_redact_tokens_without_the_header_scheme(
    tmp_path, header, scheme, token
):
    settings = Settings(
        "desktop",
        "linux",
        "http://localhost:8000",
        "local/model",
        None,
        headers={header: f"{scheme} {token}"},
    )
    stream = io.StringIO()
    error = f"Rejected credential: {token}"
    with diagnostics(stream, settings.secrets):
        print(error)
    assert stream.getvalue() == "Rejected credential: [redacted]\n"
    files = RunFiles(tmp_path / "run", "desktop", settings.secrets)
    files.finish("failed", error=error)
    saved = json.loads((files.directory / "result.json").read_text())
    assert saved["error"] == "Rejected credential: [redacted]"


def test_upstream_tool_error_is_a_failure():
    assert (
        response_error(
            {
                "output": [
                    {"type": "function_call_output", "output": '{"error":"unsupported action"}'}
                ]
            }
        )
        == "unsupported action"
    )
    assert response_error({"output": []}) is None


@pytest.mark.parametrize("output", [{"success": False}, {"terminated": True, "status": "failure"}])
def test_native_computer_output_failure_is_not_success(output):
    assert response_error({"output": [{"type": "computer_call_output", "output": output}]})


def test_terminal_output_preserves_result_path_with_long_chinese_summary():
    result = terminal_result(
        {"status": "completed", "summary": "完" * 10000, "result_file": "/tmp/result.json"}
    )
    assert result["result_file"] == "/tmp/result.json"
    assert len(json.dumps(result, ensure_ascii=False).encode()) < 4096


def test_artifacts_cancellation_and_interrupted_status(tmp_path, png):
    files = RunFiles(tmp_path / "run", "desktop", ("model-secret",))
    with exclusive_lock(files.directory / "active.lock"):
        files.save()
        assert read_status(files.directory)["status"] == "running"
        assert request_cancel(files.directory)["status"] == "cancel_requested"
        files.event("response", summary="model-secret")
        saved = files.screenshot(png)
        assert saved.read_bytes().startswith(b"\x89PNG")
        if os.name != "nt":
            assert saved.stat().st_mode & 0o777 == 0o600
    assert read_status(files.directory)["status"] == "interrupted"
    assert "model-secret" not in (files.directory / "events.jsonl").read_text()
    files.finish("cancelled")
    assert request_cancel(files.directory)["status"] == "cancelled"
    assert read_status(files.directory)["status"] == "cancelled"


def test_screenshot_rejects_invalid_image_and_existing_output(tmp_path):
    files = RunFiles(tmp_path / "run", "desktop")
    with pytest.raises(Exception):
        files.screenshot(b"not an image")
    with pytest.raises(FileExistsError):
        RunFiles(tmp_path / "run", "desktop")
