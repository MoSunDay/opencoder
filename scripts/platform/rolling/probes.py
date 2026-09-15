"""Readiness requires a real, durable WASM execution in the candidate."""
import hashlib
import time
import json
import socket
from pathlib import Path
from .state import atomic_bytes

WASM = bytes.fromhex("0061736d0100000001040160000003020100070a01065f737461727400000a040102000b")


def resource_paths(settings):
    # Runtime's workdir config is authoritative for mounted client resources.
    paths = {}
    for path in [Path.home() / ".config/opencoder/config.json",Path.home() / ".opencoder/opencoder.json",
            Path.home() / ".opencoder/config.json",settings.agent_workdir / "opencoder.json",
            settings.agent_workdir / ".opencoder/config.json"]:
        config = json.loads(path.read_text()) if path.exists() else {}
        for section, key in (("agent", "agents_dir"), ("dag", "wasm_dir")):
            if config.get(section, {}).get(key):
                paths[section] = Path(config[section][key])
    return list(paths.values())


def resources(settings, operations):
    config = json.loads((settings.server_workdir / "opencoder.json").read_text())
    for section, endpoint in (("agent", "/api/agents/nfs"), ("dag", "/api/dag/wasm/nfs")):
        expected = config.get(section, {}).get("nfs", {})
        if not expected.get("enabled"):
            continue
        if expected.get("read_only", True) is not True:
            raise ValueError("smooth releases require read-only resource exports")
        status = operations.http(settings.resource_url, endpoint)
        status = status.get("status", status)
        if not status.get("running") or not status.get("read_only"):
            raise ValueError(f"resource export is unavailable or writable: {section}")
        if expected.get("port") and status["port"] != expected["port"]:
            raise ValueError("resource export changed port")
        with socket.create_connection(("127.0.0.1", status["port"]), timeout=5):
            pass
    for root in resource_paths(settings):
        if not root.is_dir():
            raise ValueError(f"runtime resource root is unavailable: {root}")
        # Directory iteration forces an actual filesystem/NFS read.
        next(root.iterdir(), None)


def spec():
    return {"name": "release-probe", "steps": [{"name": "execute", "kind": {
        "type": "wasm", "command": "release-probe.wasm"}}]}


def probe_id(record, public=False):
    suffix = hashlib.sha256(record["id"].encode()).hexdigest()[:32]
    return f"dag-probe-{'public-' if public else ''}{suffix}"


def candidate(settings, record, operations, seconds):
    root = Path(record["runtime_data"])
    atomic_bytes(root / "dag/_modules/release-probe.wasm", WASM, 0o444)
    endpoint = f"http://127.0.0.1:{record['runtime_port']}"
    inventory = operations.wait(lambda: operations.http(endpoint, "/inventory"), seconds)
    if inventory.get("runtime_id") != record["id"] or inventory["build"]["git_commit"] != record["manifest"]["commit"]:
        raise ValueError("candidate runtime identity or compiled commit differs from release")
    node_id = inventory["registration"]["id"]
    identifier = probe_id(record)
    # Creation time comes from the durable release record, never from a retry.
    assignment = {"index": {"id": identifier, "kind": "dag", "node_id": node_id,
        "created_at": record["created_at"], "status": "pending"},
        "request": {"id": identifier, "kind": "dag", "input": {}, "node_id": node_id},
        "definition": spec()}
    receipt = operations.http(endpoint, "/rpc", "POST", {"operation": "create", "assignment": assignment})
    if receipt["status"] >= 300:
        raise RuntimeError(f"candidate probe rejected: {receipt}")

    def finished():
        view = operations.http(endpoint, "/inventory")
        index = next((i for i in view["indexes"] if i["id"] == identifier), None)
        if index and index["status"] in ("error", "interrupted", "cancelled"):
            raise ValueError(f"candidate probe failed: {index}")
        return index and index["status"] == "done" and view["snapshot"]["ready"]
    operations.wait(finished, seconds)
    return node_id


def ready(settings, record, node_id, operations, seconds):
    resources(settings, operations)
    endpoint = f"http://127.0.0.1:{record['server_port']}"
    def complete_index():
        nodes = operations.http(endpoint, "/api/nodes")["nodes"]
        current = next((n for n in nodes if n["id"] == node_id), None)
        if not current or not current["online"] or not current.get("snapshot", {}).get("ready"):
            return False
        return operations.http(endpoint, "/api/ready")["ready_nodes"] >= 1
    operations.wait(complete_index, seconds)


def public(settings, record, operations, seconds):
    # Sending HUP confirms the reload request; the new workers may still be
    # starting. Verify the public version within the readiness budget.
    operations.wait(lambda: operations.http(settings.public_url, "/api/admin/release")["instance_release"] == record["id"],seconds)
    identifier = probe_id(record, public=True)
    operations.http(settings.public_url, "/api/executions", "POST", {
        "id": identifier, "kind": "dag", "input": {"definition": spec()}})
    def finished():
        reply = operations.http(settings.public_url, f"/api/executions/{identifier}")
        # Inspection uses the shared five-field index plus runtime-owned detail.
        phase = reply.get("execution", reply.get("index", reply)).get("status")
        if phase in ("error", "interrupted", "cancelled"):
            raise ValueError(f"public execution probe failed: {phase}")
        return phase == "done"
    operations.wait(finished, seconds)
