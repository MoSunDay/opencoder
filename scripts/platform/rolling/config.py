"""Deployment settings live in the existing opencoder.json configuration."""
from dataclasses import dataclass, fields
from pathlib import Path
import json
import re
from urllib.parse import urlparse


@dataclass(frozen=True)
class Settings:
    state_dir: Path
    server_workdir: Path
    server_data: Path
    agent_workdir: Path
    token_file: Path
    bin_dir: Path = Path("/usr/local/bin")
    node_name: str = "local"
    server_user: str = "opencoder-server"
    public_url: str = "http://127.0.0.1:18081"
    listen: str = "127.0.0.1:18081"
    host_port: int = 18082
    resource_port: int = 18084
    port_base: int = 19000
    max_runs: int = 20
    min_memory_mb: int = 1024
    nginx_include: Path = Path("/etc/nginx/conf.d/opencoder-platform.conf")
    systemd_dir: Path = Path("/etc/systemd/system")
    legacy_agent_data: Path | None = None
    legacy_server_unit: str = "opencoder-server.service"
    legacy_agent_unit: str = "opencoder-agent.service"

    @property
    def host_url(self):
        return f"http://127.0.0.1:{self.host_port}"

    @property
    def resource_url(self):
        return f"http://127.0.0.1:{self.resource_port}"


def load(path):
    raw = json.loads(Path(path).read_text())["deployment"]
    unknown = set(raw) - {field.name for field in fields(Settings)}
    if unknown:
        raise ValueError(f"unknown deployment settings: {sorted(unknown)}")
    for key in ("state_dir", "server_workdir", "server_data", "agent_workdir",
                "token_file", "bin_dir", "nginx_include", "systemd_dir", "legacy_agent_data"):
        if raw.get(key) is not None:
            raw[key] = Path(raw[key])
            if not raw[key].is_absolute() or any(c in str(raw[key]) for c in "\n\r%"):
                raise ValueError(f"{key} must be an absolute path without control characters or %")
    settings = Settings(**raw)
    if not re.fullmatch(r"[a-z_][a-z0-9_-]*", settings.server_user):
        raise ValueError("invalid server service account")
    if settings.max_runs < 1 or settings.min_memory_mb < 1:
        raise ValueError("capacity and memory reserve must be positive")
    host, port = settings.listen.rsplit(":", 1)
    if host not in ("127.0.0.1", "0.0.0.0") or not port.isdigit():
        raise ValueError("listen must be an IPv4 bind address and port")
    if not all(0 < int(p) < 65536 for p in (port, settings.host_port, settings.resource_port, settings.port_base)):
        raise ValueError("ports must be between 1 and 65535")
    if urlparse(settings.public_url).scheme not in ("http", "https"):
        raise ValueError("public_url must use HTTP or HTTPS")
    return settings
