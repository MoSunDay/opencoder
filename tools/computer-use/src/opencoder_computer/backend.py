"""Only the upstream SDK owns model dispatch and desktop actions."""

import importlib.metadata

from . import CUA_REVISION
from .config import Settings
from .model import NativeCoordinates, check_dependencies, generation_options, probe_model


def versions() -> dict:
    output = {"source_revision": CUA_REVISION}
    for package in ("opencoder-computer", "cua-agent", "cua-computer"):
        try:
            output[package] = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            output[package] = "not installed"
    return output


class CuaBackend:
    def __init__(self, settings: Settings, hooks=None):
        from computer import Computer
        from cua_agent import ComputerAgent
        from cua_agent.computers.cua import cuaComputerHandler

        self.computer = Computer(
            os_type=settings.os_type,
            use_host_computer_server=True,
            api_base_url=settings.endpoint,
            api_headers=settings.headers,
            telemetry_enabled=False,
        )

        # The upstream handler hardcodes Linux; adapt OS metadata and use
        # only Cua's public desktop commands for Windows Unicode input.
        class DesktopHandler(cuaComputerHandler):
            async def get_environment(self):
                return {"macos": "mac"}.get(settings.os_type, settings.os_type)

            async def type(self, text):
                if settings.os_type == "windows":
                    # Native pynput typing corrupts CJK on the tested Windows layout.
                    # Use Cua's own Unicode clipboard and native hotkey commands.
                    await self.interface.set_clipboard(text)
                    await self.interface.hotkey("ctrl", "v")
                else:
                    await super().type(text)

        self.handler = DesktopHandler(self.computer)
        callbacks = (
            [NativeCoordinates(self.screenshot, settings.os_type)]
            if settings.native_tool_calls else []
        )
        if hooks is not None:
            callbacks.append(hooks)
        self.agent = ComputerAgent(
            model=settings.model,
            tools=[self.handler],
            api_base=settings.api_base,
            api_key=settings.api_key,
            callbacks=callbacks,
            only_n_most_recent_images=1 if settings.native_tool_calls else 3,
            telemetry_enabled=False,
            max_retries=0,
            instructions="Verify the requested outcome on the desktop before reporting completion. "
            "If a capability is unavailable or the task fails, "
            "describe the limitation.",
            **generation_options(settings.native_tool_calls, settings.os_type),
        )
        check_dependencies(self.agent, settings.native_tool_calls)

    async def connect(self) -> None:
        await self.computer.run()
        interface = self.computer.interface
        send_command = interface._send_command

        async def checked_command(command, params=None):
            response = await send_command(command, params)
            # Several upstream mouse/keyboard methods discard success=False.
            # Preserve server refusals at the shared transport boundary.
            if response.get("success") is False or response.get("error"):
                raise RuntimeError(f"Cua {command}: {response.get('error', 'command refused')}")
            return response

        interface._send_command = checked_command
        self.handler.interface = interface

    async def screenshot(self) -> bytes:
        return await self.computer.interface.screenshot()

    async def run(self, task: str):
        from cua_agent.agent import get_json

        async for chunk in self.agent.run(task):
            yield get_json(chunk)

    async def check_model(self, image: bytes) -> dict:
        return await probe_model(self.agent, image)

    async def close(self) -> None:
        interface = getattr(self.computer, "_interface", None)
        if interface is not None:
            # Upstream disconnect deliberately keeps host WebSockets open.
            # This CLI owns its connection and must release it before exit.
            socket = getattr(interface, "_ws", None)
            interface.force_close()
            if socket is not None:
                await socket.close()
        await self.computer.disconnect()

    async def describe(self) -> dict:
        dimensions = await self.computer.interface.get_screen_size()
        server = await self.computer.interface._send_command("version")
        environment = "unknown"
        try:
            environment = await self.computer.interface.get_desktop_environment()
        except RuntimeError:
            pass  # Optional upstream metadata command; screenshot readiness is required.
        return {
            "versions": versions(),
            "model_loop": type(self.agent.agent_loop).__name__,
            "model_capabilities": self.agent.get_capabilities(),
            "screen": dimensions,
            "server_version": server.get("package", "unknown"),
            "desktop_environment": environment,
            "desktop_actions": "provided by the remote Cua server; refusals are preserved",
        }
