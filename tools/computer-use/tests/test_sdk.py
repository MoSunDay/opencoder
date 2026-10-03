import asyncio
from dataclasses import replace

import pytest

from opencoder_computer.backend import CuaBackend
from opencoder_computer.runner import doctor, execute
from opencoder_computer.state import read_status, request_cancel


@pytest.mark.parametrize(
    "os_type,environment", [("linux", "linux"), ("windows", "windows"), ("macos", "mac")]
)
async def test_real_cua_loop_connects_model_to_remote_desktop(
    service, remote_settings, tmp_path, os_type, environment
):
    settings = replace(remote_settings, os_type=os_type)
    backend = CuaBackend(settings)
    assert await backend.handler.get_environment() == environment
    result = await execute(settings, "Click once and verify.", tmp_path / "run", tmp_path / "locks")
    assert result["status"] == "completed", result
    assert result["summary"] == "Click verified."
    assert result["actions"] == 2
    clicks = [item for item in service.commands if item["command"] == "left_click"]
    assert clicks == [{"command": "left_click", "params": {"x": 16, "y": 12}}]
    assert len(service.requests) == 3
    assert '"image"' in str(service.requests).replace("'", '"')
    assert all(item["Authorization"] == "Bearer desktop-secret" for item in service.headers)
    assert len(list((tmp_path / "run/screenshots").glob("*.png"))) >= 4
    assert all(socket.closed for socket in service.sockets)


async def test_doctor_reports_real_sdk_versions_and_remote_screen(service, remote_settings):
    result = await doctor(remote_settings)
    assert result["status"] == "ready"
    assert result["screen"] == {"width": 320, "height": 240}
    assert result["desktop_environment"] == "test-desktop"
    assert result["server_version"] == "0.3.46"
    assert result["versions"]["cua-agent"] == "0.8.4"
    assert result["model_loop"] == "AnthropicHostedToolsConfig"
    assert not service.requests


async def test_real_sdk_websocket_fallback_keeps_headers_and_closes(service, remote_settings):
    service.rest_available = False
    result = await doctor(remote_settings)
    assert result["status"] == "ready"
    assert service.sockets
    await asyncio.wait_for(service.closed.wait(), 1)
    assert all(socket.closed for socket in service.sockets)
    assert all(item["Authorization"] == "Bearer desktop-secret" for item in service.headers)


async def test_backend_close_waits_until_client_socket_is_closed(service, remote_settings):
    service.rest_available = False
    backend = CuaBackend(remote_settings)
    await backend.connect()
    socket = backend.computer.interface._ws
    assert socket.state.name == "OPEN"
    await backend.close()
    assert socket.state.name == "CLOSED"


async def test_doctor_rejects_corrupt_screenshots(service, remote_settings):
    from PIL import UnidentifiedImageError

    service.png = b"not an image"
    with pytest.raises(UnidentifiedImageError):
        await doctor(remote_settings)


async def test_doctor_timeout_has_a_readable_error(service, remote_settings):
    service.desktop_stall = True
    with pytest.raises(RuntimeError, match="desktop check deadline exceeded"):
        await doctor(remote_settings, timeout=0.05)


async def test_native_backend_refusal_cannot_report_completed(service, remote_settings, tmp_path):
    service.refuse = "left_click"
    result = await execute(remote_settings, "Click.", tmp_path / "run", tmp_path / "locks")
    assert result["status"] == "failed"
    assert "unsupported by native backend" in result["error"]
    assert len(service.requests) == 2


async def test_action_limit_stops_before_next_remote_action(service, remote_settings, tmp_path):
    result = await execute(
        remote_settings, "Click.", tmp_path / "run", tmp_path / "locks", max_actions=1
    )
    assert result["status"] == "action_limit"
    assert result["actions"] == 1
    assert not any(item["command"] == "left_click" for item in service.commands)


async def test_model_error_is_redacted_and_persisted(service, remote_settings, tmp_path):
    service.failure = True
    result = await execute(remote_settings, "Click.", tmp_path / "run", tmp_path / "locks")
    assert result["status"] == "failed"
    assert "invalid model credentials" in result["error"]
    assert "model-secret" not in (tmp_path / "run/result.json").read_text()


async def test_timeout_closes_connections_and_releases_target(service, remote_settings, tmp_path):
    service.stall = True
    result = await execute(
        remote_settings, "Click.", tmp_path / "run", tmp_path / "locks", timeout=0.1
    )
    assert result["status"] == "timed_out"
    assert read_status(tmp_path / "run")["status"] == "timed_out"
    assert (await doctor(remote_settings))["status"] == "ready"


async def test_cancel_during_model_wait_stops_and_persists(service, remote_settings, tmp_path):
    service.stall = True
    task = asyncio.create_task(
        execute(remote_settings, "Click.", tmp_path / "run", tmp_path / "locks")
    )
    for _ in range(200):
        if service.requests:
            break
        await asyncio.sleep(0.01)
    assert service.requests
    assert request_cancel(tmp_path / "run")["status"] == "cancel_requested"
    result = await task
    assert result["status"] == "cancelled"
    assert not any(item["command"] == "left_click" for item in service.commands)


async def test_concurrent_runs_cannot_control_same_target(service, remote_settings, tmp_path):
    service.stall = True
    first = asyncio.create_task(
        execute(remote_settings, "Click.", tmp_path / "first", tmp_path / "locks")
    )
    try:
        for _ in range(200):
            if service.requests:
                break
            await asyncio.sleep(0.01)
        assert service.requests
        with pytest.raises(RuntimeError, match="already in use"):
            await execute(remote_settings, "Click.", tmp_path / "second", tmp_path / "locks")
        assert not (tmp_path / "second").exists()
        request_cancel(tmp_path / "first")
        assert (await first)["status"] == "cancelled"
    finally:
        if not first.done():
            first.cancel()
            await first


@pytest.mark.parametrize("timeout", [0, -1, float("inf"), float("nan")])
async def test_invalid_limits_do_not_create_run(remote_settings, tmp_path, timeout):
    with pytest.raises(ValueError):
        await execute(
            remote_settings, "Click.", tmp_path / "run", tmp_path / "locks", timeout=timeout
        )
    assert not (tmp_path / "run").exists()
