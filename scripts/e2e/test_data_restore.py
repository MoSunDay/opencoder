#!/usr/bin/env python3
"""Real Server + Agent backup/isolated-restore acceptance."""

from __future__ import annotations

import hashlib
import http.client
import json
import os
import pathlib
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "platform"))
import data_archive  # noqa: E402


def free_port() -> int:
    with socket.socket() as stream:
        stream.bind(("127.0.0.1", 0))
        return stream.getsockname()[1]


def request(port: int, token: str, method: str, path: str, body: object | None = None) -> tuple[int, dict]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
    payload = None if body is None else json.dumps(body).encode()
    headers = {"Authorization": f"Bearer {token}"}
    if payload is not None:
        headers["Content-Type"] = "application/json"
    try:
        connection.request(method, path, body=payload, headers=headers)
        response = connection.getresponse()
        raw = response.read()
        value = json.loads(raw) if raw else {}
        return response.status, value
    finally:
        connection.close()


def until(label: str, action, timeout: float = 30) -> object:
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            last = action()
            if last:
                return last
        except (OSError, TimeoutError, json.JSONDecodeError):
            pass
        time.sleep(0.1)
    raise AssertionError(f"timeout waiting for {label}; last={last!r}")


def start(log: pathlib.Path, *command: str) -> tuple[subprocess.Popen, object]:
    stream = log.open("wb")
    process = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT)
    return process, stream


def stop(process: subprocess.Popen | None, stream: object | None, timeout: float = 40) -> None:
    if process is not None and process.poll() is None:
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
            raise
    if stream is not None:
        stream.close()


def tree_fingerprint(root: pathlib.Path) -> dict[str, tuple[str, int]]:
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and not path.is_symlink():
            relative = path.relative_to(root).as_posix()
            result[relative] = (hashlib.sha256(path.read_bytes()).hexdigest(), path.stat().st_mode & 0o777)
    return result


def main() -> None:
    binary_dir = pathlib.Path(
        os.environ.get("PLATFORM_BIN_DIR", "/data00/rust-build/cargo/opencoder-platform/debug")
    )
    server_binary = binary_dir / "opencoder-server"
    agent_binary = binary_dir / "opencoder-agent"
    if not server_binary.is_file() or not agent_binary.is_file():
        raise SystemExit(f"build opencoder-server and opencoder-agent first under {binary_dir}")

    with tempfile.TemporaryDirectory(prefix="opencoder-data-restore-") as raw:
        root = pathlib.Path(raw)
        work = root / "work"
        (work / ".opencoder").mkdir(parents=True)
        (work / ".opencoder/ap.json").write_text('{"mode":"off"}\n', encoding="utf-8")
        token = secrets.token_urlsafe(32)
        token_file = root / "token"
        token_file.write_text(token + "\n", encoding="utf-8")
        token_file.chmod(0o600)
        server_data = root / "server-data"
        node_data = root / "node-data"
        backup = root / "backup"
        restored = root / "restored"
        execution_id = "agent-backup-restore"

        server = agent = None
        server_log = agent_log = None
        try:
            port = free_port()
            server, server_log = start(
                root / "server.log",
                str(server_binary),
                "--host", "127.0.0.1",
                "--port", str(port),
                "--workdir", str(work),
                "--data-dir", str(server_data),
                "--token-file", str(token_file),
            )
            until("server", lambda: request(port, token, "GET", "/api/nodes")[0] == 200)
            agent, agent_log = start(
                root / "agent.log",
                str(agent_binary),
                "--remote", f"http://127.0.0.1:{port}",
                "--name", "backup-node",
                "--workdir", str(work),
                "--data-dir", str(node_data),
                "--token-file", str(token_file),
                "--no-dag",
            )
            node_id = until(
                "ready node",
                lambda: next(
                    (
                        node["id"]
                        for node in request(port, token, "GET", "/api/nodes")[1]["nodes"]
                        if node.get("online") and node.get("snapshot", {}).get("ready")
                    ),
                    None,
                ),
            )
            status, accepted = request(
                port,
                token,
                "POST",
                "/api/executions",
                {"id": execution_id, "kind": "agent", "node_id": node_id, "input": {"prompt": ""}},
            )
            assert status == 202, accepted
            until(
                "idle execution",
                lambda: request(port, token, "GET", f"/api/executions/{execution_id}")[1]
                .get("execution", {})
                .get("status")
                == "idle",
            )
            assert request(port, token, "POST", "/api/admin/drain", {})[0] == 200
            status, stopped = request(
                port,
                token,
                "POST",
                f"/api/executions/{execution_id}/commands",
                {"action": "interrupt", "input": {}},
            )
            assert status == 200, stopped
            until(
                "interrupted index",
                lambda: request(port, token, "GET", f"/api/executions/{execution_id}")[1]
                .get("execution", {})
                .get("status")
                == "interrupted",
            )
            stop(agent, agent_log)
            agent = agent_log = None
            stop(server, server_log)
            server = server_log = None

            source_before = {
                "server": tree_fingerprint(server_data),
                "node": tree_fingerprint(node_data),
            }
            data_archive.backup(server_data, {"node-a": node_data}, backup)
            data_archive.restore(backup, restored)
            assert source_before == {
                "server": tree_fingerprint(server_data),
                "node": tree_fingerprint(node_data),
            }

            restored_port = free_port()
            server, server_log = start(
                root / "restored-server.log",
                str(server_binary),
                "--host", "127.0.0.1",
                "--port", str(restored_port),
                "--workdir", str(work),
                "--data-dir", str(restored / "server"),
                "--token-file", str(token_file),
            )
            until("restored server", lambda: request(restored_port, token, "GET", "/api/nodes")[0] == 200)
            agent, agent_log = start(
                root / "restored-agent.log",
                str(agent_binary),
                "--remote", f"http://127.0.0.1:{restored_port}",
                "--name", "restored-node",
                "--workdir", str(work),
                "--data-dir", str(restored / "nodes/node-a"),
                "--token-file", str(token_file),
                "--no-dag",
            )
            until(
                "restored node online",
                lambda: any(
                    node.get("online")
                    for node in request(restored_port, token, "GET", "/api/nodes")[1]["nodes"]
                ),
            )
            detail = until(
                "restored historical detail",
                lambda: (
                    body
                    if (status := request(restored_port, token, "GET", f"/api/executions/{execution_id}"))[0]
                    == 200
                    and (body := status[1]).get("execution", {}).get("status") == "interrupted"
                    else None
                ),
            )
            assert detail["execution"]["node_id"] == node_id
            assert source_before == {
                "server": tree_fingerprint(server_data),
                "node": tree_fingerprint(node_data),
            }
            print(
                "PASS real Server+Agent frozen backup, isolated restore, historical local detail, "
                "and original-source preservation"
            )
        finally:
            stop(agent, agent_log)
            stop(server, server_log)


if __name__ == "__main__":
    main()
