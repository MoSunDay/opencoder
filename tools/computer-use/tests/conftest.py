import asyncio
import base64
import io
import json
from dataclasses import dataclass, field

import pytest
import pytest_asyncio
from aiohttp import web
from PIL import Image

from opencoder_computer.config import Settings


@pytest.fixture(autouse=True)
def direct_loopback_requests(monkeypatch):
    # Test external boundaries locally, independent of the developer's proxy.
    for name in (
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("LITELLM_LOCAL_MODEL_COST_MAP", "true")
    monkeypatch.setenv("LITELLM_LOCAL_ANTHROPIC_BETA_HEADERS", "true")


@pytest.fixture
def png():
    stream = io.BytesIO()
    Image.new("RGB", (320, 240), color="white").save(stream, format="PNG")
    return stream.getvalue()


@pytest.fixture
def settings():
    return Settings(
        "desktop",
        "linux",
        "http://localhost:8000",
        "anthropic/claude-sonnet-4-5",
        None,
        "model-secret",
        {"Authorization": "Bearer desktop-secret"},
    )


@dataclass
class Service:
    png: bytes
    base: str = ""
    commands: list = field(default_factory=list)
    requests: list = field(default_factory=list)
    headers: list = field(default_factory=list)
    refuse: str | None = None
    failure: bool = False
    stall: bool = False
    sockets: list = field(default_factory=list)
    rest_available: bool = True
    desktop_stall: bool = False
    closed: asyncio.Event = field(default_factory=asyncio.Event)
    chat_outputs: list | None = None

    def command(self, payload):
        self.commands.append(payload)
        name = payload["command"]
        if name == self.refuse:
            return {"success": False, "error": "unsupported by native backend"}
        values = {
            "version": {"protocol": 1, "package": "0.3.46"},
            "get_screen_size": {"size": {"width": 320, "height": 240}},
            "get_desktop_environment": {"environment": "test-desktop"},
            "screenshot": {"image_data": base64.b64encode(self.png).decode()},
        }
        return {"success": True, **values.get(name, {})}

    async def rest(self, request):
        self.headers.append(dict(request.headers))
        if self.desktop_stall:
            await asyncio.sleep(1)
        if not self.rest_available:
            return web.Response(status=404, text="REST unavailable")
        return web.Response(text="data: " + json.dumps(self.command(await request.json())))

    async def websocket(self, request):
        self.headers.append(dict(request.headers))
        socket = web.WebSocketResponse()
        await socket.prepare(request)
        self.sockets.append(socket)
        async for message in socket:
            await socket.send_json(self.command(json.loads(message.data)))
        await socket.close()
        self.closed.set()
        return socket

    async def model(self, request):
        import asyncio

        self.requests.append(await request.json())
        if self.stall:
            await asyncio.sleep(1)
        if self.failure:
            return web.json_response(
                {
                    "type": "error",
                    "error": {
                        "type": "authentication_error",
                        "message": "invalid model credentials",
                    },
                },
                status=401,
            )
        step = len(self.requests)
        if self.chat_outputs is not None:
            message = self.chat_outputs[min(step - 1, len(self.chat_outputs) - 1)]
            return web.json_response({
                "id": f"chat-{step}", "object": "chat.completion", "created": 1,
                "model": "glm-5.3-flash",
                "choices": [{"index": 0, "message": {"role": "assistant", **message},
                             "finish_reason": (
                                 "tool_calls" if message.get("tool_calls") else "stop"
                             )}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20},
            })
        if step <= 2:
            action = (
                {"action": "screenshot"}
                if step == 1
                else {"action": "left_click", "coordinate": [16, 12]}
            )
            content = [
                {"type": "tool_use", "id": f"tool-{step}", "name": "computer", "input": action}
            ]
        else:
            content = [{"type": "text", "text": "Click verified."}]
        return web.json_response(
            {
                "id": f"msg-{step}",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-5",
                "content": content,
                "stop_reason": "tool_use" if step <= 2 else "end_turn",
                "stop_sequence": None,
                "usage": {"input_tokens": 10, "output_tokens": 10},
            }
        )


@pytest_asyncio.fixture
async def service(png):
    boundary = Service(png)
    application = web.Application()
    application.router.add_post("/desktop/cmd", boundary.rest)
    application.router.add_get("/desktop/ws", boundary.websocket)
    application.router.add_post("/model/{tail:.*}", boundary.model)
    runner = web.AppRunner(application)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    boundary.base = f"http://127.0.0.1:{site._server.sockets[0].getsockname()[1]}"
    yield boundary
    for socket in boundary.sockets:
        await socket.close()
    await runner.cleanup()


@pytest.fixture
def remote_settings(service):
    return Settings(
        "desktop",
        "linux",
        service.base + "/desktop",
        "anthropic/claude-sonnet-4-5",
        service.base + "/model",
        "model-secret",
        {"Authorization": "Bearer desktop-secret"},
    )


@pytest.fixture
def cli_config(tmp_path, remote_settings):
    config = tmp_path / "computer.json"
    (tmp_path / "model.key").write_text(remote_settings.api_key)
    (tmp_path / "headers.json").write_text(json.dumps(remote_settings.headers))
    config.write_text(
        json.dumps(
            {
                "model": {
                    "name": remote_settings.model,
                    "api_base": remote_settings.api_base,
                    "api_key_file": "model.key",
                },
                "targets": {
                    "desktop": {
                        "os": "linux",
                        "url": remote_settings.endpoint,
                        "headers_file": "headers.json",
                    }
                },
            }
        )
    )
    (tmp_path / "task.txt").write_text("Click once and verify.")
    return config
