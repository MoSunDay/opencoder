import asyncio
import json
import sys
from dataclasses import replace

import pytest

from opencoder_computer.backend import CuaBackend
from opencoder_computer.config import parse_settings
from opencoder_computer.runner import doctor, execute


def click(x, y):
    return {"content": None, "tool_calls": [{
        "id": "native-click", "type": "function",
        "function": {"name": "computer", "arguments": json.dumps({
            "action": "left_click", "coordinate": [x, y],
        })},
    }]}


def native(settings):
    return replace(settings, model="openai/glm-5.3-flash", native_tool_calls=True)


async def test_native_tool_request_executes_cua_converted_coordinates(
    service, remote_settings, tmp_path
):
    service.chat_outputs = [click(600, 500), {"content": "Click verified."}]
    result = await execute(native(remote_settings), "Click once.", tmp_path / "run",
                           tmp_path / "locks")
    assert result["status"] == "completed", result
    assert result["actions"] == 1
    assert result["summary"] == "Click verified."
    assert [i for i in service.commands if i["command"] == "left_click"] == [
        {"command": "left_click", "params": {"x": 192, "y": 128}}
    ]
    for request in service.requests:
        assert request["tools"][0]["function"]["name"] == "computer"
        assert request["parallel_tool_calls"] is False
        text = json.dumps(request["messages"])
        assert "1000/320" in text and "1000/256" in text
        assert "NEVER type, x, y" in text
    assert all(socket.closed for socket in service.sockets)


async def test_model_probe_checks_coordinates_without_desktop_actions(service, remote_settings):
    service.chat_outputs = [{"content": "The image is white."}, click(781, 755)]
    result = await doctor(native(remote_settings), check_model=True)
    assert result["model_check"] == {
        "vision": "passed", "actions_executed": 0,
        "tool_format": "passed", "coordinates": "passed",
    }
    assert not any(c["command"] in {"left_click", "type_text", "press_key"}
                   for c in service.commands)
    assert len(service.requests) == 2


async def test_model_probe_rejects_raw_pixel_coordinates(service, remote_settings):
    service.chat_outputs = [{"content": "White screenshot."}, click(499, 290)]
    with pytest.raises(RuntimeError, match="coordinates"):
        await doctor(native(remote_settings), check_model=True)
    assert not any(c["command"] == "left_click" for c in service.commands)


async def test_model_probe_rejects_empty_vision_response(service, remote_settings):
    service.chat_outputs = [{"content": ""}]
    with pytest.raises(RuntimeError, match="screenshot description"):
        await doctor(native(remote_settings), check_model=True)
    assert len(service.requests) == 1


@pytest.mark.parametrize("message", [
    {"content": '<tool_call>computer<arg_key>action</arg_key></tool_call>'},
    {"content": None, "tool_calls": [{"function": {
        "name": "computer", "arguments": '{"type":"click","x":100,"y":100}'}}]},
    click("500", "500"),
    click(1001, 500),
])
async def test_native_malformed_response_stops_before_any_action(
    service, remote_settings, tmp_path, message
):
    service.chat_outputs = [message]
    result = await execute(native(remote_settings), "Click.", tmp_path / "run", tmp_path / "locks")
    assert result["status"] == "failed"
    assert result["actions"] == 0
    assert not any(c["command"] == "left_click" for c in service.commands)


@pytest.mark.parametrize("refuse", [None, "set_clipboard"])
async def test_windows_unicode_typing_uses_cua_clipboard_and_preserves_refusal(
    service, remote_settings, tmp_path, refuse
):
    text = "OpenCoder 中文验收"
    service.refuse = refuse
    service.chat_outputs = [{"tool_calls": [{"id": "type-1", "type": "function", "function": {
        "name": "computer", "arguments": json.dumps({"action": "type", "text": text}),
    }}]}, {"content": "Typing verified."}]
    result = await execute(replace(native(remote_settings), os_type="windows"), "Type Chinese.",
                           tmp_path / "run", tmp_path / "locks")
    commands = [c for c in service.commands if c["command"] in {
        "set_clipboard", "hotkey", "type_text",
    }]
    assert "wheel ticks" in json.dumps(service.requests[0])
    assert commands[0] == {"command": "set_clipboard", "params": {"text": text}}
    if refuse:
        assert result["status"] == "failed"
        assert len(commands) == 1
    else:
        assert result["status"] == "completed"
        assert commands[1] == {"command": "hotkey", "params": {"keys": ["ctrl", "v"]}}


async def test_cli_model_probe_outputs_ready_without_input_actions(service, cli_config):
    config = json.loads(cli_config.read_text())
    config["model"].update(name="openai/glm-5.3-flash", native_tool_calls=True)
    cli_config.write_text(json.dumps(config))
    service.chat_outputs = [{"content": "White screen."}, click(781, 755)]
    process = await asyncio.create_subprocess_exec(
        sys.executable, "-m", "opencoder_computer", "--config", str(cli_config),
        "doctor", "--target", "desktop", "--check-model",
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )
    stdout, stderr = await asyncio.wait_for(process.communicate(), 30)
    assert process.returncode == 0, stderr.decode()
    assert len(stdout.splitlines()) == 1
    assert json.loads(stdout)["model_check"]["actions_executed"] == 0
    assert not any(c["command"] == "left_click" for c in service.commands)


def test_native_tool_mode_rejects_non_generic_cua_loop(remote_settings):
    with pytest.raises(ValueError, match="generic vision loop"):
        CuaBackend(replace(remote_settings, native_tool_calls=True))


def test_native_tool_mode_requires_boolean_config():
    document = {"model": {"name": "openai/glm-5.3-flash", "native_tool_calls": "true"},
                "targets": {"win": {"os": "windows", "url": "http://localhost:8000"}}}
    with pytest.raises(ValueError, match="boolean"):
        parse_settings(document, "win", lambda _: "")
