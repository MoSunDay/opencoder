"""Versioned systemd units and a fixed Nginx ingress."""
import json
import os
from pathlib import Path
import shutil
from .state import atomic_bytes, write


def argument(value):
    value = str(value)
    if "\n" in value or "\r" in value:
        raise ValueError("invalid systemd argument")
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"').replace("%", "%%").replace("$", "$$") + '"'


def service(command, description, runtime=False, user="root", workdir=None):
    directory = str(workdir or "/")
    if not directory.startswith("/") or any(c in directory for c in "\n\r"):
        raise ValueError("service working directory must be an absolute path")
    return f"""[Unit]
Description={description}
After=network-online.target remote-fs.target opencoder-resources.service
Wants=network-online.target

[Service]
Type=simple
User={user}
WorkingDirectory={directory.replace("%", "%%")}
ExecStart={" ".join(argument(a) for a in command)}
Restart=on-failure
RestartSec=1s
TimeoutStopSec=infinity
SendSIGKILL=no
KillMode=mixed
KillSignal=SIGTERM
LimitNOFILE=1048576
{("Delegate=yes" if runtime else "")}

[Install]
WantedBy=multi-user.target
"""


def inherited_environment(settings, unit):
    """Carry existing service credential/tool lookup configuration unchanged."""
    lines = []
    paths = [settings.systemd_dir / unit, *sorted((settings.systemd_dir / f"{unit}.d").glob("*.conf"))]
    for path in paths:
        if not path.exists():
            continue
        section = None
        for line in path.read_text().splitlines():
            stripped = line.strip()
            if stripped.startswith('['):
                section = stripped
            if section == '[Service]' and stripped.startswith(('Environment=', 'EnvironmentFile=', 'PassEnvironment=', 'UnsetEnvironment=')):
                if stripped.endswith('\\'):
                    raise ValueError(f"multiline service environment requires normalization: {path}")
                lines.append(line)
    return '\n'.join(lines)


def prepare(settings, bundle, record):
    root = settings.state_dir / "releases" / record["id"]
    installed = root / "bundle"
    if not installed.exists():
        root.mkdir(parents=True, exist_ok=True)
        stage = root / "bundle.staging"
        if stage.exists():
            # A killed copy has no running unit and is safe to replace.
            shutil.rmtree(stage)
        shutil.copytree(bundle, stage)
        for path in stage.rglob("*"):
            if path.is_file():
                with path.open("rb") as stream:
                    os.fsync(stream.fileno())
        for directory in sorted((p for p in stage.rglob("*") if p.is_dir()),key=lambda p:len(p.parts),reverse=True) + [stage]:
            fd = os.open(directory,os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        stage.rename(installed)
        fd = os.open(root,os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    from .manifest import verify
    if verify(installed) != record["manifest"]:
        raise ValueError("retained bundle differs from the immutable release record")
    installed.chmod(0o755)
    (installed / "bin").chmod(0o755)
    for binary in (installed / "bin").iterdir():
        binary.chmod(0o755)
    node_id = (settings.state_dir / "host/node-id").read_text().strip()
    runtime_data = Path(record["runtime_data"])
    if (runtime_data / "node-id").exists() and (runtime_data / "node-id").read_text().strip() != node_id:
        raise ValueError("runtime node identity differs from the stable host")
    atomic_bytes(runtime_data / "node-id", node_id.encode())
    write(runtime_data / "host-binding.json", {"database": str(settings.state_dir / "host/host.db"), "runtime_id": record["id"]})
    agent = installed / "bin/opencoder-agent"
    commands = {
        record["runtime_unit"]: [agent, "--workdir", settings.agent_workdir, "--data-dir", runtime_data,
            "--max-runs", 65535, "--token-file", settings.token_file, "runtime", "--port", record["runtime_port"]],
    }
    for unit, command in commands.items():
        is_runtime = unit == record["runtime_unit"]
        content = service(command, unit, is_runtime, "root" if is_runtime else settings.server_user,
            settings.agent_workdir if is_runtime else settings.server_workdir)
        content = content.replace("Type=simple", "Type=simple\n" + inherited_environment(settings, settings.legacy_agent_unit))
        if unit == record["runtime_unit"]:
            # Use configured resource roots; don't add a Server lifecycle edge.
            from .probes import resource_paths
            paths = resource_paths(settings)
            if paths:
                content = content.replace("[Service]", "RequiresMountsFor=" + " ".join(argument(p) for p in paths) + "\n\n[Service]")
        atomic_bytes(settings.systemd_dir / unit, content.encode(), 0o644)
    prepare_host(settings, record)
    prepare_server(settings, record)


def prepare_server(settings, record):
    root = settings.state_dir / "releases" / record["id"]
    platform = {"release_id": record["id"], "state_dir": str(settings.state_dir),
        "host_service": settings.host_url, "resource_service": settings.resource_url}
    release_config = root / "release.json"
    atomic_bytes(release_config, (json.dumps(platform) + "\n").encode(), 0o644)
    command = [root / "bundle/bin/opencoder-server", "--host", "127.0.0.1", "--port", record["server_port"],
        "--workdir", settings.server_workdir, "--release-config", release_config,
        "--data-dir", settings.server_data, "--token-file", settings.token_file]
    content = service(command,record["server_unit"],user=settings.server_user,workdir=settings.server_workdir)
    content = content.replace("Type=simple", "Type=simple\n" + inherited_environment(settings,settings.legacy_server_unit))
    atomic_bytes(settings.systemd_dir / record["server_unit"],content.encode(),0o644)


def prepare_host(settings, record):
    agent = settings.state_dir / "releases" / record["id"] / "bundle/bin/opencoder-agent"
    command = [agent, "--name", settings.node_name, "--workdir", settings.agent_workdir,
        "--data-dir", settings.state_dir / "host", "--remote", settings.public_url,
        "--max-runs", settings.max_runs, "--token-file", settings.token_file,
        "host", "--port", record["host_port"], "--standby"]
    atomic_bytes(settings.systemd_dir / record["host_unit"], service(command, record["host_unit"],workdir=settings.agent_workdir).encode(), 0o644)


def activate_launchers(settings, record, operations):
    operations.run("python3", str(Path(__file__).parents[1] / "install_bundle.py"),
        "--bundle", str(settings.state_dir / "releases" / record["id"] / "bundle"),
        "--dest-dir", str(settings.bin_dir))


def validate(settings, record, operations):
    operations.run("systemd-analyze", "verify", *(str(settings.systemd_dir / record[key])
        for key in ("runtime_unit", "host_unit", "server_unit")))


def nginx(settings, server_port, host_port):
    # No worker_shutdown_timeout: old workers retain accepted requests.
    def location(port):
        return f"""location / {{
        proxy_pass http://127.0.0.1:{port};
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $opencoder_platform_connection;
        proxy_buffering off;
        proxy_request_buffering off;
        proxy_cache off;
        proxy_next_upstream off;
        proxy_read_timeout 1d;
        proxy_send_timeout 1d;
        add_header X-Accel-Buffering no always;
    }}"""
    return f"""map $http_upgrade $opencoder_platform_connection {{ default upgrade; '' close; }}
server {{ listen {settings.listen}; client_max_body_size 2m; {location(server_port)} }}
server {{ listen 127.0.0.1:{settings.host_port}; client_max_body_size 2m; {location(host_port)} }}
"""


def switch_ingress(settings, record, operations):
    previous = settings.nginx_include.read_bytes() if settings.nginx_include.exists() else None
    content = nginx(settings, record["server_port"], record["host_port"]).encode()
    atomic_bytes(settings.nginx_include, content, 0o644)
    try:
        operations.run("nginx", "-t")
        operations.run("systemctl", "reload", "nginx")
    except Exception:
        if previous is not None:
            atomic_bytes(settings.nginx_include, previous, 0o644)
        raise
