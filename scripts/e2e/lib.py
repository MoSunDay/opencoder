"""Shared helpers for the opencoder e2e suite.

Design: every assertion goes through `Counter.check` so we get a uniform
pass/fail tally with a clear name. Model-dependent contracts use *soft*
asserts (record a skip on model non-cooperation); deterministic store
contracts (fork copy, bundle roundtrip, resume history-load, plan read-only)
use *hard* asserts.

The deep-assertion workhorse is `show_json`: it shells out to
`opencoder session show <id> --json` (the CLI enabler added alongside this
suite) and returns `{meta, messages, subagent_tasks}`. This decouples the
e2e from storage internals (no sqlite coupling, no hash-path derivation).
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
from dataclasses import dataclass, field
from typing import Any

DEFAULT_BIN = "/data/caches/opencoder-target/release/opencoder"
AUTH_PATH = os.path.expanduser("~/.local/share/opencoder/auth.json")

# Tool markers emitted by the headless event printer (see cli/src/run.rs).
RE_SESSION_ID = re.compile(r"\[session ([0-9A-Z]+)\]")
RE_FORK_ID = re.compile(r"\[forked [0-9A-Z]+ \u2192 ([0-9A-Z]+)\]")
TOOL_START = "\u25b8"  # ▸
SUBAGENT_START = "subagent ["
COMPACTION_MARK = "[context compacted]"


def resolve_bin(arg: str | None) -> str:
    if arg:
        return arg
    env = os.environ.get("OPENCODER_BIN")
    if env:
        return env
    return DEFAULT_BIN


def ensure_auth() -> str:
    """Return the ZHIPU_API_KEY, loading from auth.json if needed."""
    key = os.environ.get("ZHIPU_API_KEY")
    if key:
        return key
    if os.path.isfile(AUTH_PATH):
        with open(AUTH_PATH) as f:
            data = json.load(f)
        return data["zhipuai-coding-plan"]["key"]
    raise SystemExit("FAIL: set ZHIPU_API_KEY or install opencoder auth.json")


def make_config(
    *,
    reasoning_effort: str = "medium",
    max_tokens: int = 16384,
    context_threshold: int = 100_000,
    tail_turns: int = 3,
    reserved: int = 4000,
    api_key: str,
) -> dict[str, Any]:
    """A glm5.2 config; compaction turned down only via context_threshold."""
    return {
        "model": "zhipuai-coding-plan/glm-5.2",
        "provider": {
            "base_url": "https://open.bigmodel.cn/api/coding/paas/v4",
            "api_key": "{ZHIPU_API_KEY}",
        },
        "reasoning_effort": reasoning_effort,
        "max_tokens": max_tokens,
        "compaction": {
            "auto": True,
            "context_threshold": context_threshold,
            "tail_turns": tail_turns,
            "reserved": reserved,
        },
    }


def seed_workdir(cfg: dict[str, Any]) -> str:
    """Create a temp workdir, write opencoder.json, return the path."""
    d = tempfile.mkdtemp(prefix="opencoder_e2e_")
    with open(os.path.join(d, "opencoder.json"), "w") as f:
        json.dump(cfg, f)
    return d


def write_file(workdir: str, name: str, content: str) -> None:
    with open(os.path.join(workdir, name), "w") as f:
        f.write(content)


@dataclass
class Counter:
    passed: int = 0
    failed: int = 0
    skipped: int = 0
    notes: list[str] = field(default_factory=list)

    def check(self, name: str, cond: bool, detail: str = "") -> None:
        if cond:
            self.passed += 1
            print(f"  ok: {name}")
        else:
            self.failed += 1
            extra = f" ({detail})" if detail else ""
            print(f"  FAIL: {name}{extra}")

    def soft(self, name: str, cond: bool, detail: str = "") -> None:
        """Model-dependent contract: skip (not fail) when the model doesn't cooperate."""
        if cond:
            self.passed += 1
            print(f"  ok: {name}")
        else:
            self.skipped += 1
            extra = f" ({detail})" if detail else ""
            print(f"  -- SKIP: {name}{extra}")

    def note(self, msg: str) -> None:
        self.notes.append(msg)
        print(f"  note: {msg}")

    def __add__(self, other: "Counter") -> "Counter":
        return Counter(
            passed=self.passed + other.passed,
            failed=self.failed + other.failed,
            skipped=self.skipped + other.skipped,
            notes=self.notes + other.notes,
        )

    def summary(self, label: str) -> None:
        print(f"\n[{label}] {self.passed} passed, {self.failed} failed, {self.skipped} skipped")


def run(bin_path: str, args: list[str], timeout: int = 900) -> tuple[int, str]:
    """Run the binary; return (returncode, combined stdout+stderr)."""
    try:
        p = subprocess.run(
            [bin_path] + args,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or "") if isinstance(e.stdout, str) else ""
        err = (e.stderr or "") if isinstance(e.stderr, str) else ""
        return 124, out + err + f"\nTIMEOUT after {timeout}s"


def run_prompt(
    bin_path: str, workdir: str, prompt: str, *flags: str, timeout: int = 900
) -> tuple[int, str]:
    """Run a headless one-shot prompt in a workdir."""
    return run(bin_path, ["--workdir", workdir, *flags, prompt], timeout=timeout)


def models_text(bin_path: str, workdir: str) -> str:
    _, out = run(bin_path, ["--workdir", workdir, "models"], timeout=60)
    return out


def show_json(bin_path: str, workdir: str, sid: str) -> dict[str, Any]:
    """Deep inspection: meta + all message blocks + subagent tasks."""
    rc, out = run(bin_path, ["--workdir", workdir, "session", "show", sid, "--json"], timeout=60)
    if rc != 0:
        raise RuntimeError(f"session show --json failed (rc={rc}): {out[-400:]}")
    return json.loads(out)


def extract_session_id(log: str) -> str | None:
    m = RE_SESSION_ID.search(log)
    return m.group(1) if m else None


def extract_fork_id(log: str) -> str | None:
    m = RE_FORK_ID.search(log)
    return m.group(1) if m else None


def assistant_text(session: dict[str, Any]) -> str:
    """Concatenate all assistant Text blocks (full history)."""
    parts = []
    for m in session.get("messages", []):
        if m.get("role") == "assistant":
            for b in m.get("blocks", []):
                if b.get("kind") == "text":
                    parts.append(b.get("text", ""))
    return "\n".join(parts)


def all_text(session: dict[str, Any]) -> str:
    """All Text blocks regardless of role (for content-integrity checks)."""
    parts = []
    for m in session.get("messages", []):
        for b in m.get("blocks", []):
            if b.get("kind") == "text":
                parts.append(b.get("text", ""))
    return "\n".join(parts)


def message_roles(session: dict[str, Any]) -> list[str]:
    return [m.get("role") for m in session.get("messages", [])]


def session_list(bin_path: str, workdir: str) -> str:
    """List sessions for a workdir; returns stdout."""
    _, out = run(bin_path, ["--workdir", workdir, "session", "list"], timeout=60)
    return out


def session_delete(bin_path: str, workdir: str, sid: str) -> str:
    """Delete a session; returns stdout."""
    _, out = run(bin_path, ["--workdir", workdir, "session", "delete", sid], timeout=60)
    return out


def config_show(bin_path: str, workdir: str) -> str:
    """Show merged config as JSON; returns stdout."""
    _, out = run(bin_path, ["--workdir", workdir, "config", "show"], timeout=30)
    return out


def has_reasoning_blocks(session: dict[str, Any]) -> bool:
    """True if any assistant message has a Reasoning content block
    (interleaved thinking: reasoning_content persisted on tool turns)."""
    for m in session.get("messages", []):
        if m.get("role") == "assistant":
            for b in m.get("blocks", []):
                if b.get("kind") == "reasoning":
                    return True
    return False
