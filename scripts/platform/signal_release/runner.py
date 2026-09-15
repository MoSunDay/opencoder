"""The root systemd job survives Server/Host retirement and writes a receipt."""
import argparse
import json
import re
import time
import uuid
from pathlib import Path
from rolling import config, deployment
from rolling.io import Operations
from rolling.state import Journal, locked, write


def parse_instance(instance):
    match = re.fullmatch(r"(deploy|rollback)--([A-Za-z0-9_-]{1,64})", instance)
    if not match:
        raise ValueError("invalid signal job instance")
    return match.groups()


def target_for(settings, action, origin, journal):
    pending = None
    if action == "deploy":
        pending = json.loads((settings.state_dir / "signal-pending.json").read_text())
    # A killed controller may have persisted the new pointer before finishing
    # ingress verification. Only that same unfinished operation can resume.
    resume_deploy = (action == "deploy" and journal["previous"] == origin
        and journal["candidate"] == pending["release_id"]
        and journal["phase"] in ("switching", "verifying"))
    resume_rollback = (action == "rollback" and journal.get("rollback_from") == origin
        and journal["phase"] == "rolling_back")
    if journal["current"] != origin and not (resume_deploy or resume_rollback):
        raise ValueError("signal came from a superseded server; no release was started")
    if action == "deploy":
        return pending["release_id"], Path(pending["bundle"])
    target = journal["current"] if journal["phase"] == "rolled_back" else journal["previous"]
    if not target:
        raise ValueError("no compatible previous release is recorded")
    return target, None


def run(settings, instance, operations, seconds=90):
    action, origin = parse_instance(instance)
    receipt = {"attempt": uuid.uuid4().hex, "action": action, "origin": origin,
        "phase": "starting", "started_at": time.time_ns()}
    path = settings.state_dir / "signal-receipts" / (instance + ".json")
    write(path, receipt)
    try:
        with locked(settings.state_dir):
            target, bundle = target_for(settings, action, origin, Journal(settings.state_dir).data)
            receipt.update(target=target, phase="running")
            write(path, receipt)
            result = (deployment.deploy(settings, bundle, operations, seconds) if action == "deploy"
                      else deployment.rollback(settings, operations, seconds))
            if result["current"] != target or result["phase"] not in ("complete", "rolled_back"):
                raise RuntimeError("release did not reach its requested target")
            receipt.update(phase="complete", current=result["current"], finished_at=time.time_ns())
            write(path, receipt)
    except BaseException as error:
        receipt.update(phase="failed", failure=f"{type(error).__name__}: {error}", finished_at=time.time_ns())
        write(path, receipt)
        raise
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--instance", required=True)
    args = parser.parse_args()
    settings = config.load(args.config)
    print(json.dumps(run(settings, args.instance, Operations(settings.token_file)), indent=2))


if __name__ == "__main__":
    main()
