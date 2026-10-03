"""Pure response normalization. Images remain files, never terminal output."""

import json
import re
from contextlib import contextmanager, redirect_stderr, redirect_stdout


class RedactedStream:
    """Buffer whole diagnostic lines so secrets split across writes stay private."""

    def __init__(self, stream, secrets):
        self.stream, self.secrets, self.pending = stream, secrets, ""

    def write(self, value: str) -> int:
        self.pending += value
        while "\n" in self.pending:
            line, self.pending = self.pending.split("\n", 1)
            self.stream.write(str(redact(line, self.secrets)) + "\n")
        return len(value)

    def flush(self) -> None:
        self.stream.flush()

    def isatty(self) -> bool:
        return False

    def finish(self) -> None:
        if self.pending:
            self.stream.write(str(redact(self.pending, self.secrets)))
            self.pending = ""
        self.stream.flush()


@contextmanager
def diagnostics(stream, secrets):
    protected = RedactedStream(stream, secrets)
    try:
        with redirect_stdout(protected), redirect_stderr(protected):
            yield
    finally:
        protected.finish()


EXIT_CODES = {
    "completed": 0,
    "running": 0,
    "cancel_requested": 0,
    "failed": 1,
    "cancelled": 130,
    "timed_out": 124,
    "action_limit": 1,
    "interrupted": 1,
}


def redact(value: object, secrets: tuple[str, ...] = ()) -> object:
    if isinstance(value, dict):
        return {
            key: "[redacted]"
            if key.lower() in {"api_key", "apikey", "authorization", "cookie", "token", "password"}
            else redact(item, secrets)
            for key, item in value.items()
            if key not in {"image_url", "screenshotBase64"}
        }
    if isinstance(value, list):
        return [redact(item, secrets) for item in value]
    if isinstance(value, str):
        for secret in sorted(secrets, key=len, reverse=True):
            value = value.replace(secret, "[redacted]")
        return re.sub(r"data:image/[^\s\"']+", "[image saved separately]", value)
    return value


def response_text(chunk: dict) -> str:
    messages = []
    for item in chunk.get("output", []):
        if item.get("type") == "message" and item.get("role") == "assistant":
            for content in item.get("content", []):
                if isinstance(content, dict) and isinstance(content.get("text"), str):
                    messages.append(content["text"])
    return "\n".join(messages)


def response_error(chunk: dict) -> str | None:
    if chunk.get("error"):
        return str(chunk["error"])
    for item in chunk.get("output", []):
        if item.get("type") in {"function_call_output", "computer_call_output"}:
            output = item.get("output")
            if isinstance(output, str):
                try:
                    output = json.loads(output)
                except ValueError:
                    pass
            if isinstance(output, dict) and (output.get("error") or output.get("success") is False):
                return str(output.get("error", "Cua tool failed"))
            if (
                isinstance(output, dict)
                and output.get("terminated")
                and (output.get("status") == "failure")
            ):
                return "Cua reported task failure"
        if item.get("is_error") or item.get("error"):
            return str(item.get("error", "Cua tool failed"))
    return None


def terminal_result(result: dict) -> dict:
    """Keep CLI output below OpenCoder's 4 KiB tool-output cap."""
    output = dict(result)
    output["summary"] = str(output.get("summary", ""))[:300]
    if output.get("error"):
        output["error"] = str(output["error"])[:300]
    if len(json.dumps(output, ensure_ascii=False).encode()) > 3800:
        output = {
            key: output[key]
            for key in ("status", "result_file", "summary", "error", "actions")
            if key in output
        }
        output["summary"] = output.get("summary", "")[:100]
    return output
