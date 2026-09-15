"""The single maintenance window that introduces the stable Host and NFS owner."""
import json
import shutil
import time
from . import backup, manifest, probes, units
from .deployment import record_for, register_runtime, register_server
from .state import Journal, atomic_bytes, write


def receipt(settings, bundle):
    candidate = manifest.verify(bundle)
    if not settings.legacy_agent_data or not (settings.legacy_agent_data / "node-id").is_file():
        raise ValueError("migration requires legacy_agent_data with its persisted node-id")
    if settings.legacy_agent_data in (settings.state_dir, settings.server_data) or settings.state_dir.is_relative_to(settings.legacy_agent_data):
        raise ValueError("platform state must be separate from legacy node and server data")
    return {"release_id": candidate["release_id"], "node_id": (settings.legacy_agent_data / "node-id").read_text().strip(),
        "maintenance": "wait for active runs, tools and queues; stop legacy units; consistent backup; start independent services",
        "legacy_units": [settings.legacy_agent_unit, settings.legacy_server_unit],
        "backup": str(settings.state_dir / "backups/first-migration"),
        "preserved": [str(settings.legacy_agent_data), str(settings.server_data), str(settings.token_file)]}


def migrate(settings, bundle, operations, seconds=90):
    review = receipt(settings, bundle)
    journal = Journal(settings.state_dir)
    if journal.data["current"] and journal.data["current"] != review["release_id"]:
        raise ValueError("another release is active; migration cannot replace it")
    if journal.data.get("migration_stage") == "complete":
        probes.public(settings, journal.record(review["release_id"]), operations, seconds)
        operations.run("systemctl", "disable", settings.legacy_agent_unit, settings.legacy_server_unit)
        return journal.data
    write(settings.state_dir / "migration-receipt.json", review)
    candidate = manifest.verify(bundle)
    manifest.resources(settings, candidate)
    record = journal.data["releases"].get(candidate["release_id"])
    if record is None:
        record = record_for(settings, candidate, 0)
        record["runtime_data"] = str(settings.legacy_agent_data)
        journal.data["releases"][record["id"]] = record
        journal.data["candidate"] = record["id"]
        journal.phase("migrating")
    stage = journal.data.get("migration_stage", "waiting")
    if stage == "waiting":
        def idle():
            nodes = operations.http(settings.public_url, "/api/nodes")["nodes"]
            if len(nodes) != 1 or nodes[0]["id"] != review["node_id"]:
                raise ValueError("first migration requires the configured single local node")
            snapshot = nodes[0].get("snapshot", {})
            if snapshot.get("max_runs") != settings.max_runs:
                raise ValueError("migration max_runs must match the existing node's effective capacity")
            if snapshot.get("active_runs") != 0 or snapshot.get("pending_runs", 0) != 0:
                return False
            status = operations.http(settings.public_url, "/api/admin/drain")
            return all(n["body"].get("owned_processes") == 0 for n in status["nodes"])
        # Readiness timeout here is a wait budget, never an execution deadline.
        operations.wait(idle, seconds)
        operations.http(settings.public_url, "/api/admin/drain", "POST", {})
        if not idle():
            operations.http(settings.public_url, "/api/admin/drain", "DELETE")
            raise ValueError("work arrived before migration freeze; admission reopened, retry when idle")
        journal.data["migration_stage"] = "stopping"
        journal.save()
        stage = "stopping"
    if stage == "stopping":
        # Remove the old Server PartOf linkage before stopping that Server.
        for path in settings.systemd_dir.glob("opencoder-agent.service.d/*resources.conf"):
            original = path.read_bytes()
            destination = settings.state_dir / "backups/unit-files" / path.parent.name / path.name
            if not destination.exists():
                atomic_bytes(destination, original)
            lines = [line for line in original.decode().splitlines() if not line.startswith("PartOf=")]
            lines = [line.replace("opencoder-server.service", "opencoder-resources.service") for line in lines]
            atomic_bytes(path, ("\n".join(lines) + "\n").encode(), 0o644)
        for path in settings.systemd_dir.glob("mnt-opencoder*.mount"):
            original = path.read_bytes()
            destination = settings.state_dir / "backups/unit-files" / path.name
            if not destination.exists():
                atomic_bytes(destination, original)
            lines = [line for line in original.decode().splitlines() if not line.startswith("PartOf=")]
            atomic_bytes(path, ("\n".join(line.replace("opencoder-server.service", "opencoder-resources.service") for line in lines) + "\n").encode(), 0o644)
        operations.run("systemctl", "daemon-reload")
        operations.run("systemctl", "stop", settings.legacy_agent_unit, settings.legacy_server_unit)
        if not (settings.state_dir / "backups/first-migration/backup-manifest.json").exists():
            backup.snapshot(settings, settings.state_dir / "backups/first-migration", stopped=True)
        journal.data["migration_stage"] = "services"
        journal.save()
        stage = "services"
    if stage == "services":
        atomic_bytes(settings.state_dir / "host/node-id", review["node_id"].encode())
        # Independent resource owner uses the first compatible bundle forever
        # until an explicitly separate resource-service maintenance operation.
        resource_binary = settings.state_dir / "services/opencoder-resources"
        if not resource_binary.exists():
            atomic_bytes(resource_binary, (bundle / "bin/opencoder-server").read_bytes(), 0o755)
        resource_unit = units.service([resource_binary, "--resources", "--workdir", settings.server_workdir,
            "--data-dir", settings.state_dir / "resources", "--port", settings.resource_port,
            "--token-file", settings.token_file], "OpenCoder independent read-only resource service",
            user=settings.server_user, workdir=settings.server_workdir)
        resource_data = settings.state_dir / "resources"
        resource_data.mkdir(exist_ok=True)
        shutil.chown(resource_data, user=settings.server_user)
        resource_unit = resource_unit.replace(" remote-fs.target opencoder-resources.service", "")
        resource_unit = resource_unit.replace("Type=simple", "Type=simple\n" + units.inherited_environment(settings,settings.legacy_server_unit))
        atomic_bytes(settings.systemd_dir / "opencoder-resources.service", resource_unit.encode(), 0o644)
        units.prepare(settings, bundle, record)
        units.validate(settings, record, operations)
        operations.run("systemctl", "daemon-reload")
        operations.run("systemctl", "enable", "--now", "opencoder-resources.service", record["host_unit"])
        operations.wait(lambda: operations.http(settings.resource_url, "/api/health"), seconds)
        if not journal.data.get("migration_mounts_ready"):
            for mount in settings.systemd_dir.glob("mnt-opencoder*.mount"):
                # Stable NFS handles survive the resource-owner handoff. Keep
                # an existing mount (which may have other readers) in place;
                # readiness below verifies the new exporter and actual reads.
                operations.run("systemctl", "start", mount.name)
            journal.data["migration_mounts_ready"] = True
            journal.save()
        host_url = f"http://127.0.0.1:{record['host_port']}"
        operations.wait(lambda: operations.http(host_url, "/status"), seconds)
        register_runtime(settings, record, operations, host_url=host_url)
        operations.run("systemctl", "start", record["runtime_unit"])
        runtime_url = f"http://127.0.0.1:{record['runtime_port']}"
        operations.wait(lambda: operations.http(runtime_url, "/inventory"), seconds)
        operations.http(runtime_url, "/rpc", "POST", {"operation":"admission", "command":"reopen"})
        probes.candidate(settings, record, operations, seconds)
        operations.http(host_url, f"/runtimes/{record['id']}/activate", "POST", {})
        register_server(settings, record, operations, host_url=host_url)
        operations.http(host_url, "/activate-host", "POST", {})
        operations.run("systemctl", "start", record["server_unit"])
        server_url = f"http://127.0.0.1:{record['server_port']}"
        operations.wait(lambda: operations.http(server_url, "/api/health"), seconds)
        operations.wait(lambda: operations.http(server_url, "/api/admin/drain", "DELETE"), seconds)
        probes.ready(settings, record, review["node_id"], operations, seconds)
        journal.data.update(current=record["id"], candidate=None)
        journal.data["migration_stage"] = "switching"
        journal.phase("switching")
        stage = "switching"
    if stage == "switching":
        host_url = f"http://127.0.0.1:{record['host_port']}"
        atomic_bytes(settings.nginx_include, units.nginx(settings, record["server_port"], record["host_port"]).encode(), 0o644)
        operations.run("nginx", "-t")
        operations.run("systemctl", "enable", "--now", "nginx", record["server_unit"], record["runtime_unit"])
        operations.run("systemctl", "reload", "nginx")
        operations.http(host_url, "/commit-host", "POST", {})
        probes.public(settings, record, operations, seconds)
        units.activate_launchers(settings, record, operations)
        operations.run("systemctl", "disable", settings.legacy_agent_unit, settings.legacy_server_unit)
        journal.data["migration_stage"] = "complete"
        journal.phase("complete")
    return journal.data
