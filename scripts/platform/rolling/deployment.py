"""Validated -> warming -> ready -> switching -> verifying -> complete.

The journal records intent before every switch; replay uses the same release
and probe IDs. Retired runtimes remain independently owned systemd services.
"""
from pathlib import Path
from contextlib import contextmanager
import copy
import time
from . import ingress, manifest, probes, units
from .state import Journal
from .network.ports import first_available


def record_for(settings, bundle_manifest, ordinal):
    identifier = bundle_manifest["release_id"]
    port = first_available(settings.port_base + ordinal * 3, 3)
    return {"id": identifier, "manifest": bundle_manifest, "server_port": port,
        "runtime_port": port + 1, "host_port": port + 2,
        "runtime_data": str(settings.state_dir / "runtimes" / identifier),
        "server_unit": f"opencoder-server-{identifier}.service",
        "runtime_unit": f"opencoder-runtime-{identifier}.service",
        "host_unit": f"opencoder-host-{identifier}.service",
        "created_at": int(time.time() * 1000)}


def fresh_frontends(record, releases, reason, retirement=None):
    """Keep old HTTP bodies/RPCs alive while starting another activation."""
    used = [int(r[k]) for r in releases for k in ("host_port", "server_port", "runtime_port")]
    used.extend(h["port"] for r in releases for key in ("previous_hosts", "previous_servers") for h in r.get(key, []))
    port = first_available(max(used) + 1, 2)
    result = copy.deepcopy(record)
    result.setdefault("previous_hosts", []).append({"unit":record["host_unit"],"port":record["host_port"]})
    result.setdefault("previous_servers", []).append({"unit":record["server_unit"],"port":record["server_port"], **(retirement or {})})
    result.update(host_port=port, server_port=port + 1,
        host_unit=f"opencoder-host-{record['id']}-{reason}-{len(result['previous_hosts'])}.service",
        server_unit=f"opencoder-server-{record['id']}-{reason}-{len(result['previous_servers'])}.service")
    return result


def register_runtime(settings, record, operations, host_url=None):
    return operations.http(host_url or settings.host_url, "/runtimes", "POST", {
        "id": record["id"], "release_id": record["id"], "mode": "staged", "config": {
            "endpoint": f"http://127.0.0.1:{record['runtime_port']}",
            "data_dir": record["runtime_data"], "unit": record["runtime_unit"]}})


def register_server(settings, record, operations, enabled=True, host_url=None):
    return operations.http(host_url or settings.host_url, f"/servers/{record['id']}", "POST", {
        "id": record["id"], "url": f"http://127.0.0.1:{record['server_port']}", "enabled": enabled})


def deploy(settings, bundle, operations, seconds=90):
    journal = Journal(settings.state_dir)
    candidate = manifest.verify(bundle)
    manifest.brain_preflight(settings, candidate, journal.data["releases"].values())
    manifest.compatible(candidate, [r["manifest"] for r in journal.data["releases"].values()])
    manifest.resources(settings, candidate)
    if journal.data["phase"] == "rolling_back":
        return rollback(settings, operations, seconds)
    if not journal.data["current"]:
        raise ValueError("first migration is required; use --migrate after reviewing the migration receipt")
    identifier = candidate["release_id"]
    if journal.data["candidate"] not in (None, identifier):
        raise ValueError("another release is unfinished; resume it or roll it back first")
    if journal.data["current"] == identifier and journal.data["phase"] in ("complete", "rolled_back"):
        probes.public(settings, journal.record(identifier), operations, seconds)
        retire_server(settings, journal, operations)
        return journal.data
    retained = identifier in journal.data["releases"]
    if not retained:
        used = [int(r[k]) for r in journal.data["releases"].values() for k in ("server_port", "runtime_port", "host_port")]
        used.extend(h["port"] for r in journal.data["releases"].values() for key in ("previous_hosts", "previous_servers") for h in r.get(key, []))
        ordinal = (max(used, default=settings.port_base - 1) + 3 - settings.port_base) // 3
        record = record_for(settings, candidate, ordinal)
        record["resource_source"] = journal.record(journal.data["current"])["runtime_data"]
        journal.data["releases"][identifier] = record
    record = journal.record(identifier)
    if record["manifest"] != candidate:
        raise ValueError("release ID already belongs to another bundle")
    if journal.data["candidate"] is None:
        if retained:
            record = fresh_frontends(record, list(journal.data["releases"].values()), "activate", journal.data.get("retirement", {}).get(identifier))
            journal.data["releases"][identifier] = record
        journal.data.get("retirement", {}).pop(identifier, None)
        record["probe_epoch"] = record.get("probe_epoch", 0) + 1
        # Retrying the current release after failed rollback preparation must
        # retain the distinct rollback target recorded before that attempt.
        previous = journal.data["previous"] if journal.data["current"] == identifier else journal.data["current"]
        journal.data.update(candidate=identifier, previous=previous, failure=None,
            rollback_from=None, rollback_from_phase=None, rollback_switch_started=None)
        journal.phase("validated")
    try:
        if journal.data["phase"] in ("validated", "warming", "failed"):
            journal.phase("warming")
            units.prepare(settings, bundle, record)
            units.validate(settings, record, operations)
            operations.run("systemctl", "daemon-reload")
            operations.run("systemctl", "enable", record["runtime_unit"], record["server_unit"], record["host_unit"])
            register_runtime(settings, record, operations)
            operations.run("systemctl", "start", record["runtime_unit"])
            node_id = probes.candidate(settings, record, operations, seconds)
            # Each Host upgrade is warmed while runtimes remain independent.
            operations.run("systemctl", "start", record["host_unit"])
            host_url = f"http://127.0.0.1:{record['host_port']}"
            operations.wait(lambda: operations.http(host_url, "/status"), seconds)
            register_server(settings, record, operations)
            operations.run("systemctl", "start", record["server_unit"])
            probes.ready(settings, record, node_id, operations, seconds)
            journal.phase("ready")
        if journal.data["phase"] in ("ready", "switching"):
            manifest.brain_preflight(settings, candidate, journal.data["releases"].values())
            if journal.data["phase"] == "ready":
                journal.data["ingress_workers"] = operations.ingress_workers()
            # The transition intent is durable before changing Host or Nginx.
            journal.data["current"] = identifier
            journal.phase("switching")
            operations.http(f"http://127.0.0.1:{record['host_port']}", f"/runtimes/{identifier}/activate", "POST", {})
            operations.http(f"http://127.0.0.1:{record['host_port']}", "/activate-host", "POST", {})
            units.switch_ingress(settings, record, operations)
            operations.http(f"http://127.0.0.1:{record['host_port']}", "/commit-host", "POST", {})
            journal.phase("verifying")
        if journal.data["phase"] == "verifying":
            probes.public(settings, record, operations, seconds)
            units.activate_launchers(settings, record, operations)
            journal.data["candidate"] = None
            journal.phase("complete")
        retire_server(settings, journal, operations)
        return journal.data
    except Exception as error:
        journal.fail(error)
        if journal.data["phase"] in ("switching", "verifying"):
            rollback(settings, operations, seconds)
        elif journal.data["phase"] in ("validated", "warming", "ready"):
            # Keep current tasks and admission intact. Candidate evidence and
            # probe ownership remain available for a same-ID resume.
            journal.phase("failed")
        raise


@contextmanager
def _deferred_disable(operations):
    changed = False

    def disable(*units):
        nonlocal changed
        # A failed multi-unit disable can still have removed some links.
        changed = True
        operations.run("systemctl", "--no-reload", "disable", *units)

    try:
        yield disable
    finally:
        if changed:
            operations.run("systemctl", "daemon-reload")


def retire_server(settings, journal, operations):
    with _deferred_disable(operations) as disable:
        _retire_server(settings, journal, operations, disable)


def _retire_server(settings, journal, operations, disable):
    def retire_port(port, unit, retirement):
        try:
            if "ingress_workers" not in retirement:
                if "ingress_workers" not in journal.data:
                    raise ValueError('retirement has no recorded ingress frontier')
                retirement["ingress_workers"] = journal.data["ingress_workers"]
            retirement.setdefault("successor_port", journal.record(journal.data["current"])["server_port"])
            journal.save()
            retiring = ingress.retire(operations, f"http://127.0.0.1:{port}", retirement["ingress_workers"], retirement["successor_port"])
        except OSError:
            if not operations.inactive(unit):
                raise
            retiring = True
        disable(unit)
        return "retiring" if retiring else "waiting_for_ingress"
    for identifier, record in journal.data["releases"].items():
        for previous in record.get("previous_servers", []):
            try:
                phase = retire_port(previous["port"], previous["unit"], previous)
                previous.update(phase=phase, failure=None)
            except Exception as error:
                previous.update(phase="failed", failure=str(error))
            journal.save()
        for host in record.get("previous_hosts", []):
            disable(host["unit"])
        if identifier == journal.data["current"]:
            continue
        retirement = journal.data.setdefault("retirement", {}).setdefault(identifier, {})
        try:
            # POST returns immediately; old response bodies and RPCs drain
            # without a stop deadline. A lost response is safe to retry.
            if retirement.get("phase") != "retiring":
                phase = retire_port(record['server_port'],record['server_unit'],retirement)
            else:
                phase = "retiring"
            register_server(settings, record, operations, enabled=False)
            disable(record["server_unit"], record["host_unit"])
            retirement.update(phase=phase, failure=None)
        except Exception as error:
            retirement.update(phase="failed", failure=str(error))
        journal.save()


def rollback(settings, operations, seconds=90):
    journal = Journal(settings.state_dir)
    if journal.data["phase"] == "rolled_back":
        probes.public(settings, journal.record(journal.data["current"]), operations, seconds)
        retire_server(settings, journal, operations)
        return journal.data
    previous = journal.data["previous"]
    if not previous:
        raise ValueError("no compatible previous release is recorded")
    old = journal.record(previous)
    manifest.compatible(old["manifest"], [r["manifest"] for r in journal.data["releases"].values()])
    resuming = journal.data["phase"] == "rolling_back" or (
        journal.data["phase"] == "failed" and journal.data.get("rollback_switch_started") is False
        and journal.data.get("rollback_from") == journal.data["current"]
        and journal.data["candidate"] is None)
    if not resuming:
        # Fresh standby instances let rollback proceed while older instances
        # finish response bodies and RPCs. Runtime units are never restarted.
        old = fresh_frontends(old, list(journal.data["releases"].values()), "rollback", journal.data.get("retirement", {}).get(previous))
        journal.data["releases"][previous] = old
        journal.data.get("retirement", {}).pop(previous, None)
        old["probe_epoch"] = old.get("probe_epoch", 0) + 1
        journal.data["rollback_from"] = journal.data["current"]
        journal.data["rollback_from_phase"] = journal.data["phase"]
        journal.data["rollback_switch_started"] = False
    journal.phase("rolling_back")
    host_url = f"http://127.0.0.1:{old['host_port']}"
    try:
        units.prepare_host(settings, old)
        units.prepare_server(settings, old)
        units.validate(settings, old, operations)
        operations.run("systemctl", "daemon-reload")
        operations.run("systemctl", "enable", old["host_unit"], old["server_unit"], old["runtime_unit"])
        operations.run("systemctl", "start", old["host_unit"])
        operations.run("systemctl", "start", old["server_unit"], old["runtime_unit"])
        operations.wait(lambda: operations.http(host_url, "/status"), seconds)
        node_id = probes.candidate(settings, old, operations, seconds)
        register_server(settings, old, operations, host_url=host_url)
        probes.ready(settings, old, node_id, operations, seconds)
    except Exception as error:
        journal.fail(error)
        # A failed standby must not trap future deployments. Missing markers
        # belong to older deployers and cannot prove traffic was untouched.
        if (journal.data.get("rollback_switch_started") is False
                and journal.data.get("rollback_from_phase") in ("complete", "rolled_back", "verifying")):
            journal.data["candidate"] = None
            journal.phase("failed")
        raise
    if not journal.data.get("rollback_switch_started"):
        journal.data["ingress_workers"] = operations.ingress_workers()
    journal.data["rollback_switch_started"] = True
    journal.save()
    operations.http(host_url, f"/runtimes/{previous}/activate", "POST", {})
    operations.http(host_url, "/activate-host", "POST", {})
    journal.data.update(current=previous, candidate=None)
    journal.save()
    units.switch_ingress(settings, old, operations)
    operations.http(host_url, "/commit-host", "POST", {})
    probes.public(settings, old, operations, seconds)
    units.activate_launchers(settings, old, operations)
    journal.phase("rolled_back")
    retire_server(settings, journal, operations)
    return journal.data
