"""Pure configuration validation, with credential reads at a single I/O boundary."""

import json
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit


@dataclass(frozen=True)
class Settings:
    target: str
    os_type: str
    endpoint: str
    model: str
    api_base: str | None
    api_key: str | None = field(default=None, repr=False)
    headers: dict[str, str] = field(default_factory=dict, repr=False)
    native_tool_calls: bool = False

    @property
    def secrets(self) -> tuple[str, ...]:
        values = [self.api_key, *self.headers.values()]
        for name, value in self.headers.items():
            scheme, separator, credential = value.partition(" ")
            if (
                name.lower() in {"authorization", "proxy-authorization"}
                and separator
                and scheme.lower() in {"bearer", "basic"}
            ):
                values.append(credential.strip())
        return tuple(value for value in values if value)


def object_value(value: object, name: str) -> dict:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be a JSON object")
    return value


def text_value(value: object, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} must be a non-empty string")
    return value.strip()


def endpoint_value(value: object, name: str) -> str:
    text = text_value(value, name)
    parts = urlsplit(text)
    if parts.scheme not in {"http", "https"} or not parts.hostname:
        raise ValueError(f"{name} must be an HTTP(S) URL")
    if parts.username or parts.password or parts.query or parts.fragment:
        raise ValueError(f"{name} must not contain credentials, query parameters or fragments")
    try:
        parts.port
    except ValueError as error:
        raise ValueError(f"{name} has an invalid port") from error
    return text.rstrip("/")


def parse_settings(document: object, target_name: str, read_secret) -> Settings:
    root = object_value(document, "config")
    targets = object_value(root.get("targets"), "targets")
    if target_name not in targets:
        raise ValueError(f"unknown target: {target_name}")
    target = object_value(targets[target_name], "target")
    os_type = text_value(target.get("os"), "target.os")
    if os_type not in {"windows", "macos", "linux"}:
        raise ValueError("target.os must be windows, macos or linux")
    model = object_value(root.get("model"), "model")
    native_tool_calls = model.get("native_tool_calls", False)
    if not isinstance(native_tool_calls, bool):
        raise ValueError("model.native_tool_calls must be a boolean")
    headers = {}
    if target.get("headers_file") is not None:
        headers = object_value(
            json.loads(read_secret(text_value(target["headers_file"], "headers_file"))),
            "headers_file",
        )
        if any(
            not isinstance(key, str)
            or not isinstance(value, str)
            or "\n" in key + value
            or "\r" in key + value
            for key, value in headers.items()
        ):
            raise ValueError("headers_file must contain string header names and values")
    api_key = None
    if model.get("api_key_file") is not None:
        api_key = text_value(
            read_secret(text_value(model["api_key_file"], "api_key_file")), "API key file"
        )
    return Settings(
        target_name,
        os_type,
        endpoint_value(target.get("url"), "target.url"),
        text_value(model.get("name"), "model.name"),
        endpoint_value(model["api_base"], "model.api_base") if model.get("api_base") else None,
        api_key,
        headers,
        native_tool_calls,
    )


def load_settings(path: Path, target: str) -> Settings:
    def read_secret(name: str) -> str:
        secret_path = Path(name).expanduser()
        if not secret_path.is_absolute():
            secret_path = path.resolve().parent / secret_path
        return secret_path.read_text(encoding="utf-8").strip()

    return parse_settings(json.loads(path.read_text(encoding="utf-8")), target, read_secret)
