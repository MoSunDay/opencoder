"""One persistent deployment root; external source workspaces remain read-only inputs."""
import copy
import json
import os
from pathlib import Path
import pwd
import stat

from ..state import atomic_bytes


def layout(settings):
    root = settings.state_dir
    return {
        "server": settings.server_data,
        "server_share": settings.server_data / ".opencoder/share",
        "host": root / "host",
        "resources": root / "resources",
        "services": root / "services",
        "runtimes": root / "runtimes",
        "releases": root / "releases",
        "backups": root / "backups",
        "runners": root / "runners",
        "server_workdir": settings.server_workdir,
        "agent_workdir": settings.agent_workdir,
        "agents_export": root / "exports/agents",
        "binaries_export": root / "exports/binaries",
        "ontology_export": root / "exports/ontology",
        "agents_mount": root / "mounts/agents",
        "binaries_mount": root / "mounts/binaries",
        "workspace_mount": root / "mounts/workspace",
        "rootfs": root / "images/dag-rootfs",
        "install": root / "install/systemd",
    }


def _bind(config, section, key, path):
    target = config.setdefault(section, {})
    if target.get(key) is not None and Path(target[key]) != path:
        raise ValueError(f"{section}.{key} is managed by deployment.state_dir; "
                         f"migrate its existing data to {path} before removing the old setting")
    target[key] = str(path)


def configurations(settings, declaration):
    """Render service configuration without reading or moving application data."""
    from ..maintenance.configuration import overlay

    server_file = settings.server_workdir / "opencoder.json"
    base = json.loads(server_file.read_text()) if server_file.exists() else {}
    base.pop("deployment", None)
    for key, value in declaration.items():
        if key == "deployment":
            continue
        if isinstance(value, dict) and isinstance(base.get(key), dict):
            base[key] = {**base[key], **copy.deepcopy(value)}
        else:
            base[key] = copy.deepcopy(value)
    server = overlay(base, settings.server_config)
    agent_base = copy.deepcopy(base)
    # Server export paths are inputs to NFS, not the Node's client mount paths.
    for section, keys in [("agent", ["agents_dir", "share_dir", "nfs"]),
                          ("dag", ["binary_dir", "workspace_dir", "nfs", "workspace_nfs"]),
                          ("ontology", ["files_dir", "nfs"])]:
        for key in keys:
            agent_base.get(section, {}).pop(key, None)
        if agent_base.get(section) == {}:
            agent_base.pop(section)
    agent_file = settings.agent_workdir / "opencoder.json"
    agent = overlay(json.loads(agent_file.read_text()) if agent_file.exists() else agent_base,
                    settings.agent_config)
    paths = layout(settings)
    for config, bindings in [
        (server, [("agent", "agents_dir", "agents_export"),
                  ("agent", "share_dir", "server_share"),
                  ("dag", "binary_dir", "binaries_export"),
                  ("ontology", "files_dir", "ontology_export")]),
        (agent, [("agent", "agents_dir", "agents_mount"),
                 ("dag", "binary_dir", "binaries_mount"),
                 ("dag", "workspace_dir", "workspace_mount"),
                 ("dag", "rootfs_dir", "rootfs")]),
    ]:
        for section, key, name in bindings:
            _bind(config, section, key, paths[name])
    # A Runtime always derives its execution directory from its immutable record.
    for config in (server, agent):
        if config.get("dag", {}).pop("data_dir", None) is not None:
            raise ValueError("dag.data_dir is managed by the Runtime; remove the repeated setting")
    for section, key in [("agent", "nfs"), ("dag", "nfs"), ("dag", "workspace_nfs")]:
        export = server.setdefault(section, {}).setdefault(key, {})
        export.setdefault("enabled", True)
        if export.get("enabled") is not True or export.get("read_only", True) is not True:
            raise ValueError(f"Server {section}.{key} must be enabled and read-only")
    server.pop("deployment", None)
    agent.pop("deployment", None)
    return agent, server


def allow_server_traversal(settings, path):
    """Expose directory traversal only inside the managed root, never external sources."""
    path = Path(path)
    if not path.is_relative_to(settings.state_dir):
        raise ValueError("service path must be inside persistent root")
    for parent in [path, *path.parents]:
        if not parent.is_relative_to(settings.state_dir):
            break
        if parent.is_symlink():
            raise ValueError("service path must not traverse a directory symlink")
        parent.chmod(stat.S_IMODE(parent.stat().st_mode) | 0o111)


def plan(settings, declaration):
    agent, server = configurations(settings, declaration)
    paths = layout(settings)
    if settings.state_dir.is_symlink():
        raise ValueError("persistent root must be a real directory, not a symlink")
    source = server.get("dag", {}).get("workspace_dir")
    if not isinstance(source, str) or not Path(source).is_absolute():
        raise ValueError("dag.workspace_dir must name the existing external Server source workspace")
    source = Path(source)
    if not source.is_dir():
        raise ValueError(f"Server source workspace must already exist: {source}")
    root = settings.state_dir.resolve()
    resolved_source = source.resolve()
    if root.is_relative_to(resolved_source) or resolved_source.is_relative_to(root):
        raise ValueError("persistent root must be outside Server source workspace")
    for path in paths.values():
        if not path.resolve().is_relative_to(root):
            raise ValueError(f"managed path escapes persistent root through a symlink: {path}")
        if any(parent.is_symlink() for parent in [path, *path.parents]
               if parent != settings.state_dir and parent.is_relative_to(settings.state_dir)):
            raise ValueError(f"managed path must not alias another directory through a symlink: {path}")
    from ..native.ontology import files_root
    bound = files_root(settings.server_data)
    if bound is not None and bound != paths["ontology_export"].resolve():
        raise ValueError("Ontology files root differs from its database binding; verify and migrate "
                         "all ready text revisions before changing the binding to "
                         + str(paths["ontology_export"]))
    from ..maintenance.mounts import name
    mounts = [{"path": str(paths[path]), "source": str(source_path),
               "port": server[section][key].get("port", port)}
              for path, source_path, section, key, port in [
                  ("agents_mount", paths["agents_export"], "agent", "nfs", 2049),
                  ("binaries_mount", paths["binaries_export"], "dag", "nfs", 2050),
                  ("workspace_mount", source, "dag", "workspace_nfs", 2051)]]
    ports = [item["port"] for item in mounts]
    if server.get("ontology", {}).get("nfs", {}).get("enabled"):
        ports.append(server["ontology"]["nfs"].get("port", 2052))
    if any(type(port) is not int or not 0 < port < 65536 for port in ports) or len(set(ports)) != len(ports):
        raise ValueError("resource NFS ports must be valid and distinct")
    for mount in mounts:
        mount["unit"] = name(mount["path"])
    return {"state_dir": str(settings.state_dir),
            "paths": {key: str(value) for key, value in paths.items()},
            "source_workspace": str(source), "mounts": mounts}


def prepare(settings, declaration):
    """Prepare files for review; never start services, mount resources or migrate data."""
    result = plan(settings, declaration)
    agent, server = configurations(settings, declaration)
    from ..maintenance.mounts import unit

    files = {settings.state_dir / "deployment.json": ({"deployment": declaration["deployment"]}, 0o600),
             settings.agent_workdir / "opencoder.json": (agent, 0o600),
             settings.server_workdir / "opencoder.json": (server, 0o600)}
    rendered = {path: ((json.dumps(value, indent=2, sort_keys=True) + "\n").encode(), mode)
                for path, (value, mode) in files.items()}
    rendered.update({Path(result["paths"]["install"]) / item["unit"]:
                     (unit(item).encode(), 0o644) for item in result["mounts"]})
    # Verify the complete plan before creating anything. Re-running preserves edits.
    for path, (data, _) in rendered.items():
        if path.is_symlink() or (path.exists() and path.read_bytes() != data):
            raise ValueError(f"prepared file already exists with different content: {path}")
    account = pwd.getpwnam(settings.server_user)
    paths = layout(settings)
    for path in paths.values():
        path.mkdir(parents=True, exist_ok=True)
    for path, (data, mode) in rendered.items():
        if not path.exists():
            atomic_bytes(path, data, mode)
    for key in ("server", "server_share", "resources", "server_workdir", "agents_export",
                "binaries_export", "ontology_export"):
        os.chown(paths[key], account.pw_uid, account.pw_gid)
        allow_server_traversal(settings, paths[key])
    for key in ("services", "releases"):
        allow_server_traversal(settings, paths[key])
    os.chown(settings.server_workdir / "opencoder.json", account.pw_uid, account.pw_gid)
    return {**result, "configuration": {"deployment": str(settings.state_dir / "deployment.json"),
                                       "server": str(settings.server_workdir / "opencoder.json"),
                                       "agent": str(settings.agent_workdir / "opencoder.json")},
            "services_started": False, "data_migrated": False}
