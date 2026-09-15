"""Never signal an older binary: Unix defaults would terminate that Server."""
import json
from rolling.state import Journal
from .controller import prefix
from .runner import parse_instance


def trigger(settings, action, operations, seconds=90):
    journal = Journal(settings.state_dir).data
    origin = journal["current"]
    instance = f"{action}--{origin}"
    parse_instance(instance)
    record = journal["releases"][origin]
    base = f"http://127.0.0.1:{record['server_port']}"
    metadata = operations.http(base, "/api/admin/release")
    if metadata.get("signal_protocol") != 1 or metadata.get("instance_release") != origin:
        raise ValueError("current Server has no signal protocol; deploy normally once before using --signal")
    if metadata.get("retiring"):
        raise ValueError("current Server is retiring")
    receipt_path = settings.state_dir / "signal-receipts" / (instance + ".json")
    before = json.loads(receipt_path.read_text()) if receipt_path.exists() else None
    unit = f"{prefix(settings)}@{instance}.service"
    # A duplicate while this job is running joins that job. An inactive job
    # must produce a new attempt; an old successful receipt cannot pass wait.
    joining = before and before["phase"] in ("starting", "running") and not operations.inactive(unit)
    operations.run("systemctl", "kill", "--kill-who=main",
                   "--signal=SIGUSR2" if action == "deploy" else "--signal=SIGUSR1", record["server_unit"])

    def completed():
        if not receipt_path.exists():
            return None
        receipt = json.loads(receipt_path.read_text())
        if before and receipt["attempt"] == before["attempt"] and not joining:
            return None
        if receipt["phase"] == "failed":
            # ValueError escapes Operations.wait; failures are not readiness
            # retries and must be reported immediately.
            raise ValueError(f"signal release failed: {receipt['failure']}; receipt: {receipt_path}")
        return receipt if receipt["phase"] == "complete" else None

    return operations.wait(completed, seconds)
