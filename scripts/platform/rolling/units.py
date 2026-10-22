"""Versioned systemd units and a fixed Nginx ingress."""
import errno
import json
import os
import re
from pathlib import Path
import shutil
import sys
import uuid
from .state import atomic_bytes, write


def argument(value):
    value = str(value)
    if "\n" in value or "\r" in value:
        raise ValueError("invalid systemd argument")
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"').replace("%", "%%").replace("$", "$$") + '"'


def service(command, description, runtime=False, user="root", workdir=None, resource=False):
    directory = str(workdir or "/")
    if not directory.startswith("/") or any(c in directory for c in "\n\r"):
        raise ValueError("service working directory must be an absolute path")
    if runtime:
        command = ['/usr/bin/unshare', '--mount', '--propagation', 'private', '--', *command]
    return f"""[Unit]
Description={description}
After=network-online.target remote-fs.target{("" if resource else " opencoder-resources.service")}
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


def install_bundle(settings, bundle, record):
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
    from .storage import allow_server_traversal
    allow_server_traversal(settings, installed / "bin")
    for binary in (installed / "bin").iterdir():
        binary.chmod(0o755)
    return installed


def prepare(settings, bundle, record):
    installed = install_bundle(settings, bundle, record)
    node_id = (settings.state_dir / "host/node-id").read_text().strip()
    runtime_data = Path(record["runtime_data"])
    if (runtime_data / "node-id").exists() and (runtime_data / "node-id").read_text().strip() != node_id:
        raise ValueError("runtime node identity differs from the stable host")
    atomic_bytes(runtime_data / "node-id", node_id.encode())
    write(runtime_data / "host-binding.json", {"database": str(settings.state_dir / "host/host.db"), "runtime_id": record["id"]})
    from .native import effective_config, runtime_config
    config = effective_config(settings.agent_workdir)
    source = config.get('dag', {}).get('rootfs_dir')
    if not source:
        raise ValueError('dag.rootfs_dir is required for every Runtime release')
    rootfs = freeze_rootfs(record, Path(source), installed / 'bin')
    workdir = runtime_config(settings, record, rootfs)
    agent = installed / "bin/opencoder-agent"
    commands = {
        record["runtime_unit"]: [agent, "--workdir", workdir, "--data-dir", runtime_data,
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


def image_programs(image):
    names = ['dag-runner', 'agent-step-runner', 'agent-session-runner']
    for name in ('opencoder', 'opencoder-cli', 'opencoder-server', 'opencoder-agent'):
        if ((image / 'usr/bin' / name).exists()
                or (name == 'opencoder-cli' and (image / 'usr/share/opencoder/codex-startup.json').exists())):
            names.append(name)
    return names


def freeze_rootfs(record, source, binaries=None, *, target=None, link_source=False):
    target = target or Path(record["runtime_data"]) / "dag/rootfs"
    if not source.is_absolute() or not source.exists():
        raise ValueError('configured DAG rootfs is unavailable')
    if source == target:
        raise ValueError('DAG image source must be outside its Runtime data')
    if source.is_symlink() or not source.is_dir():
        raise ValueError("OCI image source must be a real directory")
    if (source / 'workspace').is_symlink():
        raise ValueError("OCI workspace must be a real directory")
    if target.exists():
        if target.is_symlink() or not target.is_dir():
            raise ValueError("Runtime OCI image must be a real directory")
        if binaries:
            names = image_programs(target)
            for name in names:
                if (target / 'usr/bin' / name).read_bytes() != (binaries / name).read_bytes():
                    raise ValueError('retained DAG runner differs from the candidate release')
        return target
    target.parent.mkdir(parents=True,exist_ok=True)
    stage = target.parent / (".rootfs-stage-" + uuid.uuid4().hex)
    def ignored(directory, names):
        relative = Path(directory).relative_to(source)
        omitted = {'dev','proc','sys','tmp'} if relative == Path('.') else {'context'} if relative == Path('workspace') else set()
        return set(names) & omitted
    copied = {}
    def copy_file(source_file, destination):
        stat = os.stat(source_file)
        identity = (stat.st_dev, stat.st_ino)
        if identity in copied:
            os.link(copied[identity], destination)
        elif link_source:
            # Only an already frozen release image may use this path. Runner
            # replacements below unlink before copying, preserving old bytes.
            try:
                os.link(source_file, destination)
            except OSError as error:
                if error.errno != errno.EXDEV:
                    raise
                shutil.copy2(source_file, destination)
        else:
            shutil.copy2(source_file, destination)
        copied[identity] = destination
        return destination
    shutil.copytree(source,stage,symlinks=True,ignore=ignored,copy_function=copy_file)
    if binaries:
        names = image_programs(stage)
        for name in names:
            runner = stage / 'usr/bin' / name
            runner.unlink(missing_ok=True)
            shutil.copy2(binaries / name, runner)
    for name in ['dev','proc','sys','tmp','workspace/context']:
        directory = stage / name
        if directory.is_symlink():
            raise ValueError("OCI runtime mount must be a real directory")
        directory.mkdir(parents=True,exist_ok=True)
    synced = set()
    for path in stage.rglob('*'):
        if path.is_file() and not path.is_symlink():
            stat = path.stat()
            identity = (stat.st_dev, stat.st_ino)
            if identity in synced:
                continue
            with path.open('rb') as stream:
                os.fsync(stream.fileno())
            synced.add(identity)
    for directory in sorted((p for p in stage.rglob('*') if p.is_dir() and not p.is_symlink()),key=lambda p:len(p.parts),reverse=True) + [stage]:
        fd = os.open(directory,os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    stage.rename(target)
    fd = os.open(target.parent,os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return target


def prepare_server(settings, record):
    root = settings.state_dir / "releases" / record["id"]
    platform = {"release_id": record["id"], "state_dir": str(settings.state_dir),
        "host_service": settings.host_url, "resource_service": settings.resource_url}
    release_config = root / "release.json"
    atomic_bytes(release_config, (json.dumps(platform) + "\n").encode(), 0o644)
    from .storage import allow_server_traversal
    allow_server_traversal(settings, root)
    command = [root / "bundle/bin/opencoder-server", "--host", "127.0.0.1", "--port", record["server_port"],
        "--workdir", settings.server_workdir, "--release-config", release_config,
        "--data-dir", settings.server_data, "--token-file", settings.token_file]
    if settings.metrics_token_file:
        command.extend(["--metrics-token-file", settings.metrics_token_file])
    if not settings.auth_enabled:
        command.extend(["--auth-enabled", "false"])
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
    operations.run(sys.executable, str(Path(__file__).parents[1] / "install_bundle.py"),
        "--bundle", str(settings.state_dir / "releases" / record["id"] / "bundle"),
        "--dest-dir", str(settings.bin_dir))


def validate(settings, record, operations):
    operations.run("systemd-analyze", "verify", *(str(settings.systemd_dir / record[key])
        for key in ("runtime_unit", "host_unit", "server_unit")))


def nginx(settings, server_port, host_port):
    # No worker_shutdown_timeout: old workers retain accepted requests.
    def location(port, route="/", body_limit=""):
        return f"""location {route} {{
        {body_limit}
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
server {{ listen {settings.listen}; client_max_body_size 2m;
    {location(server_port, '~ ^/api/dag/binaries(?:/[^/]+)?$', 'client_max_body_size 48m;')}
    {location(server_port, '~ ^/api/project/todos/[^/]+/file$', 'client_max_body_size 0;')}
    {location(server_port)} }}
server {{ listen 127.0.0.1:{settings.host_port}; client_max_body_size 2m; {location(host_port)} }}
"""


def switch_host_ingress(settings, host_port, operations):
    """Follow the active Host without changing the live Server upstream.

    Journal.current may already name a failed candidate during rollback;
    preserve the actual Server configuration rather than inferring its port.
    """
    if type(host_port) is not int or not 0 < host_port < 65536:
        raise ValueError("invalid Host ingress port")
    previous = settings.nginx_include.read_text()
    marker = f"server {{ listen 127.0.0.1:{settings.host_port};"
    if previous.count(marker) != 1:
        raise ValueError("managed Host ingress block is missing or ambiguous")
    prefix, marker, host = previous.partition(marker)
    host, changed = re.subn(r"(?m)^        proxy_pass http://127\.0\.0\.1:[0-9]+;$",
                            f"        proxy_pass http://127.0.0.1:{host_port};", host)
    if changed != 1:
        raise ValueError("managed Host upstream is missing or ambiguous")
    content = prefix + marker + host
    if content != previous:
        _replace_ingress(settings, content.encode(), operations)


def switch_ingress(settings, record, operations):
    content = nginx(settings, record["server_port"], record["host_port"]).encode()
    _replace_ingress(settings, content, operations)


def _replace_ingress(settings, content, operations):
    previous = settings.nginx_include.read_bytes() if settings.nginx_include.exists() else None
    atomic_bytes(settings.nginx_include, content, 0o644)
    try:
        operations.run("nginx", "-t")
        operations.run("systemctl", "reload", "nginx")
    except Exception:
        if previous is not None:
            atomic_bytes(settings.nginx_include, previous, 0o644)
        raise
