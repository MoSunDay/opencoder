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
        raise ValueError("smooth deployment requires all four platform binaries")
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", manifest.get("release_id", "")):
        raise ValueError("bundle lacks a valid release_id; first build a handoff-capable release")
    compatibility = manifest.get("compatibility", {})
    for key in ("protocol", "data_format"):
        limits = compatibility.get(key, {})
        if limits.get("min") != 1 or limits.get("max") != 1:
            raise ValueError(f"unsupported handoff {key}; maintenance migration is required")
    info = _installer.build_info(bundle / "bin/opencoder-agent")
    if info.get("release_compatibility") != compatibility:
        raise ValueError("release compatibility does not match compiled binary")
    if manifest.get("brain_schema_version", 4) != info.get("brain_schema_version", 4):
        raise ValueError("brain schema does not match compiled binary")
    return manifest


def compatible(candidate, retained):
    for previous in retained:
        for key in ("protocol", "data_format"):
            new = candidate["compatibility"][key]
            old = previous["compatibility"][key]
            if not (new["min"] <= old["min"] <= old["max"] <= new["max"]):
                raise ValueError(f"candidate cannot read retained release {previous['release_id']} {key}")
        if candidate["protocol_version"] != previous["protocol_version"]:
            raise ValueError("fleet protocol differs from a retained runtime")


def brain_preflight(settings, candidate, releases=()):
    """Read-only supported-brain guard, before warming or changing any service.

    Worker admission repeats this check to close the race with a journal that
    changes after this preview. Incompatible fleet versions still cannot overlap.
    """
    if candidate.get("protocol_version", 0) < 10:
        return
    roots = {Path(r["runtime_data"]) for r in releases}
    if settings.legacy_agent_data:
        roots.add(settings.legacy_agent_data)
    for root in sorted(roots):
        paths = {path for kind in ("brain", "agent", "dag", "team", "todos", "project", "operator", "maintenance", "system") for path in (root / kind).glob("*/execution.json")}
        paths.update((root / "executions").glob("*.json"))
        for path in sorted(paths):
            record = json.loads(path.read_text())
            assignment = record["assignment"]
            request = assignment["request"]
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
