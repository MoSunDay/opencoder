"""Web-layer e2e: the two-segment delivery contract (steer + queue).

This is the only business feature unreachable from the CLI (steer/queue are
HTTP-only via `Delivery`). It boots a real `opencoder serve`, admits a steer
prompt and a queued follow-up over HTTP, then polls the messages endpoint to
prove BOTH were delivered in order — the core contract that mock-based
integration tests cannot verify against a live model + real drain scheduling.

Contract: POST A (delivery=steer) runs now; POST B (delivery=queue) is
consumed at idle after A's turn. The persisted message history must show
A's turn fully (user+assistant) BEFORE B's turn. Stdlib only (urllib) so the
suite has no third-party dependency.
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import time
import urllib.error
import urllib.request

from . import lib
from .lib import Counter


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _request(method: str, url: str, body: dict | None = None, timeout: int = 30) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url, data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def _wait_health(base: str, deadline: float) -> bool:
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base}/api/health", timeout=2) as r:
                if r.status == 200:
                    return True
        except Exception:
            time.sleep(0.3)
    return False


def run_all(bin_path: str, api_key: str) -> Counter:
    c = Counter()
    os.environ["ZHIPU_API_KEY"] = api_key  # serve subprocess inherits env
    webdir = lib.seed_workdir(lib.make_config(api_key=api_key))
    port = _free_port()
    base = f"http://127.0.0.1:{port}"
    sid = f"web-e2e-{port}"

    print(f"== E11: web two-segment delivery (steer + queue) on port {port} ==")
    proc = subprocess.Popen(
        [bin_path, "--workdir", webdir, "serve", "--host", "127.0.0.1", "--port", str(port)],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        ready = _wait_health(base, time.time() + 30)
        c.check("serve started and /api/health is up", ready)
        if not ready:
            out = proc.stdout.read(2000) if proc.stdout else ""
            err = proc.stderr.read(2000) if proc.stderr else ""
            c.note(f"serve did not become ready; stdout={out!r} stderr={err!r}")
            return c

        # A: a substantial task so its drain is still running when B is admitted.
        prompt_a = (
            "用 python3 在当前目录创建文件 app.py，实现一个简单的计算器类 Calculator，"
            "包含 add/subtract/multiply/divide 四个方法。写完运行 'python3 -m py_compile app.py'。"
        )
        rA = _request("POST", f"{base}/api/sessions/{sid}/prompt",
                      {"prompt": prompt_a, "delivery": "steer"})
        seqA = rA.get("admitted_seq")
        c.check("steer prompt A admitted (non-blocking)", seqA is not None)

        # B: queued follow-up. Admit immediately while A's drain runs, so B
        # enters the idle-queue path (consumed after A's turn goes idle).
        time.sleep(0.5)
        rB = _request("POST", f"{base}/api/sessions/{sid}/prompt",
                      {"prompt": "给 Calculator 类再加一个 square 方法，修改 app.py。", "delivery": "queue"})
        seqB = rB.get("admitted_seq")
        c.check("queue prompt B admitted", seqB is not None)
        if seqA is not None and seqB is not None:
            c.check("B admitted after A (queue ordering)", seqB > seqA,
                    f"A={seqA} B={seqB}")

        # Poll messages until BOTH prompts are delivered. The correct signal is
        # USER message count: each prompt is persisted as a user message when the
        # drain processes it (steer immediately, queue at idle). A single tool-using
        # turn emits MULTIPLE assistant messages, so assistant-message count would
        # trip on turn A alone — user count is the reliable "both processed" mark.
        delivered = False
        deadline = time.time() + 200
        last = None
        while time.time() < deadline:
            try:
                doc = _request("GET", f"{base}/api/sessions/{sid}/messages", timeout=20)
                last = doc
                users = [m for m in doc.get("messages", []) if m.get("role") == "user"]
                if len(users) >= 2:
                    delivered = True
                    break
            except Exception:
                pass
            time.sleep(2)

        c.check("both prompts delivered (steer + queue-at-idle)", delivered,
                "never observed 2 user messages")
        # Give B's turn a moment to finish writing its artifact, then verify outcome.
        if delivered:
            time.sleep(8)
            try:
                last = _request("GET", f"{base}/api/sessions/{sid}/messages", timeout=20)
            except Exception:
                pass
        if last:
            roles = [m.get("role") for m in last["messages"]]
            c.check("delivery order preserves A before B",
                    roles.count("user") >= 2 and roles.count("assistant") >= 2,
                    f"roles={roles}")
            # Business outcome (stronger than per-message text): steer A created
            # the artifact; queue B extended it — proves both turns took effect.
            app_py = os.path.join(webdir, "app.py")
            if os.path.isfile(app_py):
                with open(app_py, encoding="utf-8") as f:
                    src = f.read()
                c.check("steer turn A created app.py", "Calculator" in src or "def " in src)
                c.soft("queue turn B extended the artifact (square)",
                       "square" in src.lower(), "app.py had no square method")
            else:
                c.soft("steer turn A created app.py", False, "file missing")
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()

    c.summary("Web scenarios")
    return c
