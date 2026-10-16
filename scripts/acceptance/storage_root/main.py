#!/usr/bin/env python3
"""Targeted storage layout, real runc execution, version switch and rollback proof."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "smooth_release"))
from environment import Environment, until
from rolling import probes, storage


def inventory(root):
    return {str(path.relative_to(root)): (path.stat().st_ino, path.stat().st_uid,
            path.stat().st_mode, path.read_bytes() if path.is_file() else None)
            for path in [root, *root.rglob("*")]}


def execute(env, record, identifier):
    resource = probes.publish_probe(env.settings, record, env)
    env.api("/api/executions", "POST", {"id": identifier, "kind": "dag",
            "input": {"definition": probes.spec(resource)}})

    def finished():
        view = env.api("/api/executions/" + identifier)
        status = view["execution"]["status"]
        if status in ("error", "cancelled", "interrupted"):
            raise RuntimeError(f"{identifier}: {status}")
        return view if status == "done" else None

    until(finished, "actual runc probe " + identifier)
    assert env.owner(identifier) == record["id"]
    journal = Path(record["runtime_data"]) / "dag" / identifier / "execution.json"
    saved = json.loads(journal.read_text())
    process_root = Path(saved["annotations"]["dag_parent"]) / identifier
    assert process_root.is_relative_to(env.settings.state_dir)
    result = subprocess.check_output(["runc", "--root", str(process_root / "runc-state"),
                                      "list", "--format", "json"], text=True)
    assert not json.loads(result), "completed run must release its runc container"
    return {"id": identifier, "runtime": record["id"], "directory": str(process_root)}


def exercise(env):
    root = env.settings.state_dir
    source = env.resources.roots["workspace"]
    before = inventory(source)
    first = env.warm("storage-first")
    env.switch(first)
    receipt = {"first": execute(env, first, "dag-storage-first")}
    traffic, errors, stop = [], [], threading.Event()

    def monitor():
        while not stop.is_set():
            began = time.monotonic()
            try:
                ready = env.api("/api/ready")
                assert ready["ready_nodes"] == 1
                traffic.append({"at": began, "seconds": time.monotonic() - began})
            except Exception as error:
                errors.append(str(error))
            stop.wait(.2)

    thread = threading.Thread(target=monitor, daemon=True)
    thread.start()
    try:
        second = env.warm("storage-second")
        env.switch(second)
        receipt["second"] = execute(env, second, "dag-storage-second")
        env.reopen(first)
        env.switch(first)
        receipt["rollback"] = execute(env, first, "dag-storage-rollback")
        assert env.owner("dag-storage-first") == first["id"]
        assert env.owner("dag-storage-second") == second["id"]
        assert env.api("/api/executions/dag-storage-first")["execution"]["status"] == "done"
    finally:
        stop.set()
        thread.join(timeout=35)
    assert not thread.is_alive() and not errors, errors
    assert len(traffic) > 1
    maximum = max([row["seconds"] for row in traffic] +
                  [right["at"] - left["at"] for left, right in zip(traffic, traffic[1:])])
    assert maximum <= 30, maximum
    assert inventory(source) == before, "source content, permissions, owners and inode must stay fixed"
    expected = storage.layout(env.settings)
    assert all(path.is_relative_to(root) for path in expected.values())
    assert not source.is_relative_to(root), "external source is separate from managed storage"
    for path in [env.settings.server_data / "control.db", env.settings.server_data / "definitions.db",
                 root / "host/host.db", root / "resources/definitions.db",
                 *(Path(record["runtime_data"]) / "runtime.db" for record in env.records)]:
        assert path.is_file(), path
    return {**receipt, "passed": True, "state_dir": str(root), "source_unchanged": True,
            "public_failures": errors, "public_samples": len(traffic), "maximum_public_seconds": maximum}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--nginx", type=Path, required=True)
    parser.add_argument("--rootfs", type=Path, required=True)
    parser.add_argument("--data-parent", type=Path, required=True)
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.inside:
        os.execv("/usr/bin/unshare", ["unshare", "-m", "--propagation", "private", "--",
                 sys.executable, str(Path(__file__).resolve()), *sys.argv[1:], "--inside"])
    env = Environment(args.bin_dir.resolve(), args.nginx.resolve(), args.rootfs.resolve(), args.data_parent)
    result = {"passed": False}
    try:
        result = exercise(env)
    finally:
        env.close()
        (env.root / "storage-result.json").write_text(json.dumps(result, indent=2))
    print(json.dumps({"passed": result["passed"], "evidence": str(env.root)}), flush=True)


if __name__ == "__main__":
    main()
