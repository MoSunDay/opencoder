"""Bounded CLI lifecycle around the upstream Cua async generator."""

import asyncio
import io
import math
import signal
from contextlib import suppress
from pathlib import Path

from .backend import CuaBackend
from .config import Settings
from .locks import exclusive_lock, target_lock_path
from .results import redact, response_error, response_text
from .state import RunFiles


class RunStopped(BaseException):
    """Bypass Cua's tool-error recovery when the CLI must stop all further actions."""

    def __init__(self, status: str):
        self.status = status


class Hooks:
    def __init__(self, files: RunFiles, maximum: int):
        self.files = files
        self.maximum = maximum

    async def on_computer_call_start(self, item: dict) -> None:
        if (self.files.directory / "cancel.json").exists():
            raise RunStopped("cancelled")
        if self.files.result["actions"] >= self.maximum:
            raise RunStopped("action_limit")
        self.files.result["actions"] += 1
        self.files.save()
        self.files.event("action", action=item.get("action"))

    async def on_function_call_start(self, item: dict) -> None:
        await self.on_computer_call_start(item)

    async def on_screenshot(self, screenshot: bytes | str, name: str = "screenshot") -> None:
        self.files.screenshot(screenshot, name)


async def consume(backend, files: RunFiles, task: str) -> str:
    await backend.connect()
    files.screenshot(await backend.screenshot(), "initial")
    summary = ""
    async for chunk in backend.run(task):
        files.event("response", response=chunk)
        error = response_error(chunk)
        if error:
            raise RuntimeError(error)
        summary = response_text(chunk) or summary
    if not summary:
        raise RuntimeError("Cua ended without a final assistant result")
    files.screenshot(await backend.screenshot(), "final")
    return summary


async def watch_cancel(files: RunFiles, work: asyncio.Task) -> None:
    while not work.done():
        if (files.directory / "cancel.json").exists():
            work.cancel()
            return
        await asyncio.sleep(0.1)


async def execute(
    settings: Settings,
    task: str,
    directory: Path,
    lock_root: Path,
    timeout: float = 600,
    max_actions: int = 50,
    backend_factory=CuaBackend,
) -> dict:
    if not task.strip():
        raise ValueError("task must not be empty")
    if not math.isfinite(timeout) or timeout <= 0 or max_actions <= 0:
        raise ValueError("timeout and max-actions must be positive")
    with exclusive_lock(target_lock_path(settings.endpoint, lock_root)):
        files = RunFiles(directory, settings.target, settings.secrets)
        with exclusive_lock(files.directory / "active.lock"):
            files.save()
            backend = None
            work = None
            watcher = None
            handlers = {}
            stop_reason = "cancelled"
            status, summary, error = "failed", "", None
            try:
                backend = backend_factory(settings, Hooks(files, max_actions))
                work = asyncio.create_task(consume(backend, files, task))
                watcher = asyncio.create_task(watch_cancel(files, work))

                def stop(signum, frame):
                    work.cancel()

                for name in ("SIGINT", "SIGTERM"):
                    signum = getattr(signal, name, None)
                    if signum is not None:
                        handlers[signum] = signal.signal(signum, stop)
                try:
                    summary = await asyncio.wait_for(work, timeout)
                    status = "completed"
                except TimeoutError:
                    stop_reason = "timed_out"
                    status, error = stop_reason, "task deadline exceeded"
                except asyncio.CancelledError:
                    status = stop_reason
                except RunStopped as stopped:
                    status, error = stopped.status, stopped.status
            except Exception as caught:
                error = str(redact(str(caught), settings.secrets))
            finally:
                for signum, handler in handlers.items():
                    signal.signal(signum, handler)
                if watcher is not None:
                    watcher.cancel()
                    with suppress(asyncio.CancelledError):
                        await watcher
                if backend is not None:
                    try:
                        await asyncio.wait_for(backend.close(), 5)
                    except Exception as caught:
                        files.event("cleanup_error", error=str(caught))
                        if status == "completed":
                            status, error = "failed", "Cua connection cleanup failed"
            return files.finish(status, summary, error)


async def doctor(
    settings: Settings, timeout: float = 30, backend_factory=CuaBackend, *, check_model=False
) -> dict:
    from PIL import Image

    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout must be finite and positive")
    backend = backend_factory(settings)
    try:
        async with asyncio.timeout(timeout):
            await backend.connect()
            image = await backend.screenshot()
            if not image:
                raise RuntimeError("Cua returned an empty screenshot")
            with Image.open(io.BytesIO(image)) as captured:
                captured.verify()
            report = await backend.describe()
            if check_model:
                report["model_check"] = await backend.check_model(image)
            return {
                "status": "ready",
                "target": settings.target,
                "os": settings.os_type,
                "screenshot_bytes": len(image),
                **report,
            }
    except TimeoutError as error:
        raise RuntimeError("desktop check deadline exceeded") from error
    finally:
        await asyncio.wait_for(backend.close(), 5)
