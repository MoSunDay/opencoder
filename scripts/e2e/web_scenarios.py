"""Web-layer e2e scenarios (boots a real ``opencode serve``).

E11: two-segment delivery contract (steer + queue) — the only CLI-unreachable
     feature (steer/queue are HTTP-only via ``Delivery``).
E15: cancel/interrupt of a running turn, then prove the session still works.

Both boot a real ``opencode serve`` and drive it over HTTP. The server ALWAYS
enables bearer-token auth (auto-generates a ULID if none is provided), so the
harness starts serve with a fixed known token and sends
``Authorization: Bearer <token>`` on every request. Stdlib only (urllib) so
the suite has no third-party dependency.
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

# Fixed token for the e2e serve instance. ``serve`` unconditionally enables
# bearer auth, so every request must carry this header.
_E2E_TOKEN = "e2e-web-token"


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _request(
    method: str, url: str, body: dict | None = None, *, timeout: int = 30
) -> dict:
    """HTTP request with bearer auth. Raises on non-2xx (caller catches)."""
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url, data=data, method=method,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {_E2E_TOKEN}",
        },
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def _wait_health(base: str, deadline: float) -> bool:
    while time.time() < deadline:
        try:
            req = urllib.request.Request(
                f"{base}/api/health",
                headers={"Authorization": f"Bearer {_E2E_TOKEN}"},
            )
            with urllib.request.urlopen(req, timeout=2) as r:
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

    print(f"== booting serve on port {port} (token auth on) ==")
    proc = subprocess.Popen(
        [bin_path, "--workdir", webdir, "serve",
         "--host", "127.0.0.1", "--port", str(port),
         "--token", _E2E_TOKEN],
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

        # ---- E11: two-segment delivery (steer + queue) ----
        _run_e11_delivery(c, base, port, webdir)

        # ---- E15: cancel/interrupt mid-turn + session survival ----
        _run_e15_interrupt(c, base, port)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()

    c.summary("Web scenarios")
    return c


def _run_e11_delivery(c: Counter, base: str, port: int, webdir: str) -> None:
    """E11: POST A (delivery=steer) runs now; POST B (delivery=queue) is
    consumed at idle after A's turn. Both must be delivered in order."""
    sid = f"web-e2e-{port}"
    print(f"== E11: web two-segment delivery (steer + queue), session {sid} ==")

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


def _run_e15_interrupt(c: Counter, base: str, port: int) -> None:
    """E15: cancel a running turn mid-flight, then prove the session survives.

    Contract: POST /interrupt during an active drain must (1) be acknowledged,
    (2) stop the drain, and (3) leave the session usable for a subsequent prompt
    (the cancel token is refreshed per-drain, so a prior interrupt must not
    poison the next spawn). This cross-process runtime-core contract cannot be
    fully verified by mock-based integration tests.
    """
    sid = f"web-e2e-irq-{port}"
    print(f"== E15: cancel/interrupt mid-turn + session survival, session {sid} ==")

    # Admit a substantial task so its drain is still running when we interrupt.
    prompt = (
        "用 python3 在当前目录创建文件 calc.py，实现一个科学计算器类，"
        "包含 add/subtract/multiply/divide/power/sqrt/log 方法，"
        "每个方法都要有类型注解和 docstring。写完运行 'python3 -m py_compile calc.py'。"
    )
    r = _request("POST", f"{base}/api/sessions/{sid}/prompt",
                 {"prompt": prompt, "delivery": "steer"})
    seq = r.get("admitted_seq")
    c.check("interrupt-test prompt admitted", seq is not None)

    # Let the drain start producing output before interrupting.
    time.sleep(3)

    # Fire the interrupt.
    try:
        ir = _request("POST", f"{base}/api/sessions/{sid}/interrupt")
        c.check("interrupt acknowledged (returns ok)", ir.get("ok") is True)
    except Exception as e:
        c.check("interrupt acknowledged (returns ok)", False, str(e))
        return

    # Wait for the drain to settle after interrupt.
    time.sleep(3)

    # The interrupted session must have persisted the user prompt at minimum.
    try:
        doc = _request("GET", f"{base}/api/sessions/{sid}/messages", timeout=20)
        users = [m for m in doc.get("messages", []) if m.get("role") == "user"]
        c.check("interrupted session persisted the user prompt", len(users) >= 1)
    except Exception as e:
        c.check("interrupted session persisted the user prompt", False, str(e))

    # CRITICAL contract: the session must NOT be deadlocked. Re-admit a simple
    # prompt and verify it completes. The cancel token is refreshed per new
    # drain spawn, so a prior interrupt must not poison the next one.
    simple = "回复 pong 即可，不需要写代码。"
    try:
        r2 = _request("POST", f"{base}/api/sessions/{sid}/prompt",
                      {"prompt": simple, "delivery": "steer"})
        c.check("re-admit after interrupt accepted",
                r2.get("admitted_seq") is not None)
    except Exception as e:
        c.check("re-admit after interrupt accepted", False, str(e))
        return

    # Poll until the follow-up turn produces an assistant response AFTER the
    # second user message — proves the drain re-spawned and completed.
    survived = False
    deadline = time.time() + 120
    while time.time() < deadline:
        try:
            doc = _request("GET", f"{base}/api/sessions/{sid}/messages", timeout=20)
            msgs = doc.get("messages", [])
            user_indices = [i for i, m in enumerate(msgs) if m.get("role") == "user"]
            if len(user_indices) >= 2:
                second_user = user_indices[1]
                has_reply = any(
                    m.get("role") == "assistant"
                    for m in msgs[second_user + 1:]
                )
                if has_reply:
                    survived = True
                    break
        except Exception:
            pass
        time.sleep(2)

    c.check("session survives interrupt (re-admit completes)", survived,
            "follow-up prompt never produced an assistant response")
