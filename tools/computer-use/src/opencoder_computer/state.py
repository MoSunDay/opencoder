"""Private, atomic run artifacts and cancellation requests."""

import base64
import io
import json
import os
import tempfile
import time
from pathlib import Path

from .locks import exclusive_lock
from .results import redact


def write_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor, temporary = tempfile.mkstemp(dir=path.parent, prefix=".write-")
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(value, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


class RunFiles:
    def __init__(self, directory: Path, target: str, secrets: tuple[str, ...] = ()):
        self.directory = directory.resolve()
        self.directory.mkdir(parents=True, exist_ok=False, mode=0o700)
        self.secrets = secrets
        self.started = time.monotonic()
        self.image_count = 0
        self.result = {
            "run_id": self.directory.name,
            "target": target,
            "status": "running",
            "summary": "",
            "actions": 0,
            "error": None,
            "run_dir": str(self.directory),
            "result_file": str(self.directory / "result.json"),
            "events_file": str(self.directory / "events.jsonl"),
            "screenshots_dir": str(self.directory / "screenshots"),
        }

    def save(self) -> None:
        self.result["elapsed_seconds"] = round(time.monotonic() - self.started, 3)
        write_json(self.directory / "result.json", redact(self.result, self.secrets))

    def event(self, kind: str, **fields) -> None:
        record = redact({"event": kind, **fields}, self.secrets)
        descriptor = os.open(
            self.directory / "events.jsonl", os.O_APPEND | os.O_CREAT | os.O_WRONLY, 0o600
        )
        with os.fdopen(descriptor, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")

    def screenshot(self, image: bytes | str, name: str = "screen") -> Path:
        from PIL import Image

        if isinstance(image, str):
            image = base64.b64decode(image.split(",", 1)[-1], validate=True)
        with Image.open(io.BytesIO(image)) as captured:
            converted = io.BytesIO()
            captured.convert("RGB").save(converted, format="PNG")
        folder = self.directory / "screenshots"
        folder.mkdir(exist_ok=True, mode=0o700)
        self.image_count += 1
        path = folder / f"{self.image_count:04d}.png"
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(converted.getvalue())
        self.event("screenshot", name=name, path=str(path))
        return path

    def finish(self, status: str, summary: str = "", error: str | None = None) -> dict:
        self.result.update(status=status, summary=summary, error=error)
        self.save()
        self.event("finished", status=status)
        return redact(self.result, self.secrets)


def read_status(directory: Path) -> dict:
    if not directory.is_dir():
        raise ValueError("run directory does not exist")
    result = json.loads((directory / "result.json").read_text(encoding="utf-8"))
    if not isinstance(result, dict) or "status" not in result:
        raise ValueError("invalid run result")
    if result["status"] == "running":
        try:
            with exclusive_lock(directory / "active.lock"):
                result.update(status="interrupted", error="runner exited without a final result")
        except RuntimeError:
            if (directory / "cancel.json").exists():
                result["cancel_requested"] = True
    return result


def request_cancel(directory: Path) -> dict:
    result = read_status(directory)
    if result["status"] == "running":
        write_json(directory / "cancel.json", {"cancel": True})
        result["status"] = "cancel_requested"
    return result
