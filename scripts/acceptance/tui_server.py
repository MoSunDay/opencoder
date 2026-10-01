#!/usr/bin/env python3
"""PTY acceptance for /agent, /task, remote rendering and detached execution.

Usage: python3 scripts/acceptance/tui_server.py /path/to/opencoder
Requires pyte. Uses a loopback fixture and an isolated XDG data directory.
"""
import base64
import codecs
import fcntl
import http.server
import json
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import pyte


def event(seq, kind, data):
    return {"seq": seq, "kind": kind, "data": data, "ts": seq}


def append_turn(state, prompt):
    seq = len(state["events"]) + 1
    state["events"].extend([
        event(seq, "queue_consumed", {"seq": seq, "text": prompt}),
        event(seq + 1, "text_delta", {"text": "SERVER ANSWER"}),
        event(seq + 2, "done", {}),
    ])
    for role, text in [("user", prompt), ("assistant", "SERVER ANSWER")]:
        state["messages"].append({
            "id": str(len(state["messages"])), "role": role,
            "blocks": [{"kind": "text", "text": text}], "created_at": seq,
        })


def fixture(state):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def reply(self, body, status=200, kind="application/json"):
            data = json.dumps(body).encode() if kind == "application/json" else body.encode()
            self.send_response(status)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            path = urlsplit(self.path)
            if path.path == "/api/tui/agent-capabilities":
                self.reply({"capabilities": [{"id": "fixture-ops", "kind": "operator",
                                             "target": "codex", "summary": "Fixture Operator"}]})
            elif path.path.endswith("/events"):
                cursor = int(parse_qs(path.query).get("after", [0])[0])
                frames = list(state["events"])
                data = "".join(f'id: {e["seq"]}\nevent: {e["kind"]}\ndata: {json.dumps(e["data"])}\n\n'
                               for e in frames if e["seq"] > cursor)
                self.reply(data + "event: reconnect\ndata: switch\n\n", kind="text/event-stream")
            elif path.path.endswith("/events-page"):
                self.reply({"events": state["events"], "more": False})
            elif path.path.endswith("/index"):
                self.reply({"id": state["id"], "kind": "operator", "node_id": "fixture",
                            "status": "idle", "created_at": 1})
            else:
                self.reply({"error": "unknown route"}, 404)

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            state["requests"].append((self.path, body))
            if self.path == "/api/executions":
                state["id"] = body["id"]
                append_turn(state, body["input"]["prompt"])
                self.reply({"accepted": True}, 202)
            elif body["action"] == "prompt":
                append_turn(state, body["input"]["prompt"])
                self.reply({"ok": True})
            elif body["action"] == "http":
                tail = body["input"].get("tail", "")
                if tail.startswith("transcript?"):
                    chunks = []
                    for seq, message in enumerate(state["messages"], 1):
                        data = json.dumps(message).encode()
                        chunks.append({"seq": seq, "role": message["role"], "created_at": message["created_at"],
                                       "offset": 0, "next_offset": len(data), "total_bytes": len(data),
                                       "eof": True, "encoding": "base64",
                                       "bytes_b64": base64.b64encode(data).decode()})
                    self.reply({"chunks": chunks, "more": False})
                else:
                    self.reply({"questions": []})
            else:
                self.reply({"ok": True, "admitted_seq": 42})

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class Terminal:
    def __init__(self, binary, root, width, extra=()):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 34, width, 0, 0))
        env = dict(os.environ, TERM="xterm-256color", XDG_DATA_HOME=str(root / "data"))
        for key in ["OPENCODER_SERVER_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "OPENCODER_MODEL"]:
            env.pop(key, None)
        env["NO_PROXY"] = "127.0.0.1,localhost"
        self.process = subprocess.Popen([binary, "--workdir", str(root), "tui", *extra], stdin=slave,
                                        stdout=slave, stderr=slave, env=env, start_new_session=True)
        os.close(slave)
        self.screen = pyte.Screen(width, 34)
        self.stream = pyte.Stream(self.screen)
        self.decode = codecs.getincrementaldecoder("utf-8")("replace")
        self.raw = bytearray()

    def pump(self, seconds=0.15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([self.master], [], [], 0.05)[0]:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    break
                self.raw.extend(data)
                self.stream.feed(self.decode.decode(data))
        return "\n".join(self.screen.display)

    def wait(self, text, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            screen = self.pump()
            if text in screen:
                return screen
            if self.process.poll() is not None:
                break
        raise AssertionError(f"Missing {text!r} (exit={self.process.poll()}):\n"
                             f"{self.pump()}\nRaw tail: {bytes(self.raw[-2000:])!r}")

    def send(self, text):
        os.write(self.master, text.encode())

    def close(self):
        if self.master is None:
            return
        self.send("\x04")
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGTERM)
            self.process.wait(timeout=3)
        os.close(self.master)
        self.master = None


def check(binary, width):
    state = {"id": None, "events": [], "messages": [], "requests": []}
    server = fixture(state)
    with tempfile.TemporaryDirectory(prefix="opencoder-tui-acceptance-") as directory:
        root = Path(directory)
        (root / "opencoder.json").write_text(json.dumps({
            "opencoder_server": {"enabled": True, "url": f"http://127.0.0.1:{server.server_port}"},
            "network": {"proxy": None},
        }))
        terminal = Terminal(binary, root, width)
        try:
            terminal.wait("欢迎")
            terminal.send("/agent\r")
            terminal.wait("fixture-ops")
            terminal.send("fixture-ops\r")
            terminal.wait("Server 任务")
            terminal.send("literal @file\r")
            screen = terminal.wait("SERVER ANSWER")
            assert screen.count("literal @file") == 1, screen
            create = next(body for path, body in state["requests"] if path == "/api/executions")
            assert create["target"] == "codex" and create.get("node_id") is None
            assert "harness" not in create["input"]
            terminal.send("follow up\r")
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                terminal.pump()
                if any(body.get("action") == "prompt" for _, body in state["requests"]):
                    break
            assert len([r for r in state["requests"] if r[0] == "/api/executions"]) == 1
            terminal.send("/agent self\r")
            screen = terminal.wait("欢迎")
            assert "SERVER ANSWER" not in screen, screen
            terminal.send("/task\r")
            terminal.wait("fixture-ops")
            terminal.send("\x1b[B\x1b[B\r")
            terminal.wait("SERVER ANSWER")
            terminal.close()
            terminal = Terminal(binary, root, width, ("--session", state["id"], "--wrap", "codex"))
            terminal.wait("SERVER ANSWER")
            assert not any(body.get("action") == "interrupt" for _, body in state["requests"])
            assert len([r for r in state["requests"] if r[0] == "/api/executions"]) == 1
            print(f"PASS {width}x34: chooser, literal @, Server rendering, continuation, self isolation, task resume, Codex CLI resume, detach")
        finally:
            terminal.close()
            server.shutdown()


if __name__ == "__main__":
    for columns in (110, 72):
        check(str(Path(sys.argv[1]).resolve()), columns)
