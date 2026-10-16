"""Validate every retained runtime before permitting overlap."""
import importlib.util
from pathlib import Path
import re
import shutil
import json

_spec = importlib.util.spec_from_file_location("bundle_installer", Path(__file__).parents[1] / "install_bundle.py")
_installer = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_installer)


def verify(bundle):
    manifest = _installer.verify_bundle(bundle)
    if set(_installer.bundle_names(manifest)) != set(_installer.NAMES):
        raise ValueError("smooth deployment requires platform binaries and all native runners")
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", manifest.get("release_id", "")):
        raise ValueError("bundle lacks a valid release_id; first build a handoff-capable release")
    compatibility = manifest.get("compatibility", {})
    for key in ("protocol", "data_format"):
        limits = compatibility.get(key, {})
        supported = (1,) if key == "protocol" else (1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
        if limits.get("min") not in supported or limits.get("max") != limits.get("min"):
            raise ValueError(f"unsupported handoff {key}")
    info = _installer.build_info(bundle / "bin/opencoder-agent")
    if info.get("release_compatibility") != compatibility:
        raise ValueError("release compatibility does not match compiled binary")
    if manifest.get("brain_schema_version", 4) != info.get("brain_schema_version", 4):
        raise ValueError("brain schema does not match compiled binary")
    if execution_io(manifest) != execution_io(info):
        raise ValueError("execution IO compatibility does not match compiled binary")
    io_format(manifest)
    return manifest


def execution_io(manifest):
    # Releases before the reader bridge have no marker and use the legacy wire.
    value = manifest.get('execution_io', {'reader': 0, 'writer': 0})
    if (not isinstance(value, dict) or set(value) != {'reader', 'writer'}
            or any(type(v) is not int or v not in (0, 1) for v in value.values())
            or value['writer'] > value['reader']):
        raise ValueError('unsupported execution IO compatibility')
    return value


def writer_preflight(candidate, current):
    if execution_io(candidate)['writer'] > execution_io(current)['reader']:
        raise ValueError('publish the execution IO reader bridge before enabling the writer; current rollback Server cannot read the new wire')


def io_format(manifest):
    io = execution_io(manifest)
    data = manifest['compatibility']['data_format']
    if data == {'min': 10, 'max': 10}:
        if io != {'reader': 1, 'writer': 1}:
            raise ValueError('handoff format 10 requires execution IO reader/writer 1')
    elif io['writer'] != 0:
        raise ValueError('execution IO writer requires handoff format 10 so legacy controllers reject unsafe rollback')
    return data


def compatible(candidate, retained):
    reader = execution_io(candidate)['reader']
    new_format = io_format(candidate)
    for previous in retained:
        old_format = io_format(previous)
        if reader < execution_io(previous)['writer']:
            raise ValueError(f"release {candidate['release_id']} cannot read execution IO produced by retained release {previous['release_id']}; roll back to the reader bridge")
        for key in ("protocol", "data_format"):
            new = candidate["compatibility"][key]
            old = previous["compatibility"][key]
            if new != old:
                # Schema remains v39. Format 10 marks the durable new binding
                # wire so a legacy controller cannot authorize unsafe rollback.
                io_bridge = key == 'data_format' and reader == 1 and all(
                    value in ({'min': 9, 'max': 9}, {'min': 10, 'max': 10})
                    for value in (new_format, old_format))
                if not io_bridge:
                    raise ValueError(f"release {previous['release_id']} has a different {key}; use --maintenance")
        if candidate["protocol_version"] != previous["protocol_version"]:
            raise ValueError("fleet protocol differs from a retained runtime")


def overlapping(journal):
    return [r['manifest'] for r in journal['releases'].values() if not r.get('maintenance_retired')]


def brain_preflight(settings, candidate, releases=()):
    """Read-only supported-brain guard, before warming or changing any service.

    Worker admission repeats this check to close the race with a journal that
    changes after this preview. Incompatible fleet versions still cannot overlap.
    """
    if candidate.get("protocol_version", 0) < 10:
        return
    roots = {Path(r["runtime_data"]) for r in releases}
    # Bootstrap owns the legacy directory. After bootstrap, the retained
    # release registrations name the authoritative Runtime data; a preserved,
    # unregistered legacy copy is history and cannot resume work.
    if settings.legacy_agent_data and not roots:
        roots.add(settings.legacy_agent_data)
    for root in sorted(roots):
        paths = {path for kind in ("brain", "agent", "dag", "team", "todos", "project", "operator", "maintenance", "system") for path in (root / kind).glob("*/execution.json")}
        paths.update((root / "executions").glob("*.json"))
        for path in sorted(paths):
            record = json.loads(path.read_text())
            assignment = record["assignment"]
            request = assignment["request"]
            if (request['kind'] == 'dag' and assignment['index']['status'] not in ('done', 'error', 'cancelled')
                    and not (record.get('annotations') or {}).get('dag_parent')):
                raise ValueError(f"DAG migration blocked by nonterminal execution {request['id']}; terminate it with the previous runtime before upgrading")
            payload = request.get("input") or {}
            if not isinstance(payload, dict):
                payload = {}
            legacy = ((request["kind"] == "brain" and payload.get("schema_version") != candidate.get("brain_schema_version", 4))
                      or "_brain" in payload or "brain_scheduler" in payload
                      or "brain_receipt" in payload or "playbook_receipt" in payload)
            if legacy and assignment["index"]["status"] not in ("done", "error", "cancelled"):
                raise ValueError(f"brain migration blocked by nonterminal legacy execution {request['id']}; let its old runtime converge before upgrading")


def resources(settings, manifest):
    required = sum(item["bytes"] for item in manifest["files"].values()) * 2 + 256 * 1024 * 1024
    from .native import effective_config
    from .maintenance.planning.capacity import tree_bytes
    rootfs = effective_config(settings.agent_workdir).get('dag', {}).get('rootfs_dir')
    frozen = settings.state_dir / 'runtimes' / manifest['release_id'] / 'dag/rootfs'
    if rootfs and not frozen.exists():
        required += tree_bytes(Path(rootfs), {'dev', 'proc', 'sys', 'tmp', 'workspace/context'},
                               block_size=4096)
    if shutil.disk_usage(settings.state_dir).free < required:
        raise ValueError("insufficient disk space for candidate and retained releases")
    memory = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
    if int(memory["MemAvailable"].split()[0]) < settings.min_memory_mb * 1024:
        raise ValueError("insufficient memory for candidate; existing work will remain running")
    # SQLite WAL and flock require a local filesystem, never an NFS export.
    import subprocess
    for path in filter(None, (settings.state_dir, settings.server_data, settings.legacy_agent_data)):
        result = subprocess.run(["findmnt", "-n", "-o", "FSTYPE", "-T", str(path)], check=True, capture_output=True, text=True)
        filesystems = set(result.stdout.split())
        local = {"ext2", "ext3", "ext4", "xfs", "btrfs", "zfs", "bcachefs", "tmpfs", "ramfs", "overlay"}
        # A mount may appear more than once in this namespace. Every reported
        # layer must be local; duplicate records do not change that decision.
        if not filesystems or not filesystems <= local:
            raise ValueError(f"handoff database requires verified local storage: {path} ({result.stdout.strip()})")
