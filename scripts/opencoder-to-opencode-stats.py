#!/usr/bin/env python3
"""opencoder -> opencode token-usage incremental sync.

Reads opencoder per-project SQLite DBs
(~/.local/share/opencoder/<hash>/opencoder.db), aggregates assistant-message
token usage, and writes session + message + part rows into the opencode global
database (~/.local/share/opencode/opencode.db) so that opencode's usage stats
reflect opencoder consumption.

Incremental via a high-water-mark stored in
~/.local/share/opencode/.opencoder-sync.json (key: last_offset = max
sessions.updated_at seen). Pure functions, no classes.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import re
import sqlite3
import sys
from datetime import datetime, timezone

HOME = os.path.expanduser("~")
OPENCODE_DIR = os.path.join(HOME, ".local", "share", "opencode")
OPENCODE_DB = os.path.join(OPENCODE_DIR, "opencode.db")
ENCODER_DIR = os.path.join(HOME, ".local", "share", "opencoder")
STATE_FILE = os.path.join(OPENCODE_DIR, ".opencoder-sync.json")

GLOBAL_PROJECT_ID = "global"
SYNC_VERSION = "opencoder-sync"
BUSY_TIMEOUT_MS = 10000

def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()

# State (watermark) helpers
def default_state() -> dict:
    return {
        "last_offset": 0,
        "last_sync_at": None,
        "total_sessions_synced": 0,
        "total_messages_synced": 0,
    }

def load_state(path: str = STATE_FILE) -> dict:
    try:
        with open(path, "r", encoding="utf-8") as fh:
            st = json.load(fh)
    except (FileNotFoundError, ValueError):
        return default_state()
    base = default_state()
    base.update(st)
    return base

def save_state(path: str, state: dict) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(state, fh, indent=2, sort_keys=True)
    os.replace(tmp, path)

# Source discovery + reads
def find_encoder_dbs(encoder_dir: str = ENCODER_DIR) -> list:
    return sorted(glob.glob(os.path.join(encoder_dir, "*", "opencoder.db")))

def _connect_ro(db_path: str) -> sqlite3.Connection:
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        conn.execute("PRAGMA busy_timeout=%d" % BUSY_TIMEOUT_MS)
    except sqlite3.Error:
        pass
    return conn

def read_changed_sessions(db_path: str, offset: int) -> list:
    """Sessions with updated_at strictly greater than the watermark (ms)."""
    conn = _connect_ro(db_path)
    try:
        rows = conn.execute(
            "SELECT * FROM sessions WHERE updated_at > ? ORDER BY updated_at",
            (offset,),
        ).fetchall()
    finally:
        conn.close()
    return [dict(r) for r in rows]

def read_assistant_messages(db_path: str, session_id: str) -> list:
    conn = _connect_ro(db_path)
    try:
        rows = conn.execute(
            "SELECT * FROM messages "
            "WHERE session_id = ? AND role = 'assistant' "
            "  AND usage_json IS NOT NULL AND usage_json != '{}' "
            "ORDER BY created_at, seq",
            (session_id,),
        ).fetchall()
    finally:
        conn.close()
    return [dict(r) for r in rows]

def parse_usage(raw):
    """Map an opencoder usage_json blob into an opencode `tokens` dict."""
    u = json.loads(raw) if isinstance(raw, str) else dict(raw or {})
    inp = int(u.get("input_tokens", 0) or 0)
    out = int(u.get("output_tokens", 0) or 0)
    total = int(u.get("total_tokens", inp + out) or 0)
    return {
        "total": total,
        "input": inp,
        "output": out,
        "reasoning": int(u.get("reasoning_tokens", 0) or 0),
        "cache": {
            "write": int(u.get("cache_creation_tokens", 0) or 0),
            "read": int(u.get("cache_read_tokens", 0) or 0),
        },
    }

def sum_tokens(tokens_list):
    agg = {
        "total": 0, "input": 0, "output": 0, "reasoning": 0,
        "cache": {"write": 0, "read": 0},
    }
    for t in tokens_list:
        agg["total"] += t["total"]
        agg["input"] += t["input"]
        agg["output"] += t["output"]
        agg["reasoning"] += t["reasoning"]
        agg["cache"]["write"] += t["cache"]["write"]
        agg["cache"]["read"] += t["cache"]["read"]
    return agg

def parse_model(s):
    """Parse opencoder model string -> {id,providerID,variant}; glm-5.2 -> zhipuai-coding-plan."""
    if not s:
        return {"id": "unknown", "providerID": "unknown", "variant": "default"}
    mid = s.rsplit("/", 1)[-1]
    provider = "zhipuai-coding-plan" if mid == "glm-5.2" else s.split("/")[0]
    return {"id": mid, "providerID": provider, "variant": "default"}

def slugify(title, fallback):
    s = re.sub(r"[^a-z0-9]+", "-", (title or "").strip().lower()).strip("-")
    return s[:40] or fallback

def _has_tokens(t):
    return bool(
        t["input"] or t["output"] or t["total"]
        or t["cache"]["read"] or t["cache"]["write"]
    )

def compute_session_payload(session, messages):
    """Build the opencode write payload for one session, or None to skip.

    Pure: no DB access. Deterministic target ids (`enc_<source_id>`) make the
    write idempotent: re-sync deletes the prior messages/parts then re-inserts.
    """
    target_id = "enc_" + session["id"]
    model_json = parse_model(session.get("model"))

    msg_payloads = []
    tokens_list = []
    parent = None
    for msg in messages:
        tok = parse_usage(msg.get("usage_json"))
        if not _has_tokens(tok):
            continue
        msg_id = target_id + "_" + msg["id"]
        ts = int(msg.get("created_at", 0) or 0)
        msg_payloads.append({
            "msg_id": msg_id,
            "part_id": msg_id + "_sf",
            "time_created": ts,
            "msg_data": {
                "parentID": parent,
                "role": "assistant",
                "mode": msg.get("mode"),
                "agent": msg.get("agent"),
                "path": {"cwd": "/", "root": "/"},
                "cost": 0,
                "tokens": tok,
                "modelID": model_json["id"],
                "providerID": model_json["providerID"],
                "time": {"created": ts, "completed": ts},
                "finish": "stop",
            },
            "part_data": {
                "reason": "stop", "type": "step-finish", "tokens": tok, "cost": 0,
            },
        })
        tokens_list.append(tok)
        parent = msg_id

    if not msg_payloads:
        return None

    totals = sum_tokens(tokens_list)
    created = int(session.get("created_at", 0) or 0)
    updated = int(session.get("updated_at", created) or created)
    return {
        "session_id": target_id,
        "model_json": model_json,
        "totals": totals,
        "created": created,
        "updated": updated,
        "metadata": {"source": "opencoder-sync",
                     "opencoder_session_id": session["id"]},
        "title": session.get("title") or ("opencoder " + session["id"]),
        "agent": session.get("agent"),
        "msg_payloads": msg_payloads,
    }

# Column order for the opencode `session` row (must stay aligned with the
# values tuple produced by _session_values below).
_SESSION_COLS = [
    "id", "project_id", "workspace_id", "parent_id", "slug", "directory", "path",
    "title", "version", "share_url", "summary_additions", "summary_deletions",
    "summary_files", "summary_diffs", "metadata", "cost", "tokens_input",
    "tokens_output", "tokens_reasoning", "tokens_cache_read",
    "tokens_cache_write", "revert", "permission", "agent", "model",
    "time_created", "time_updated", "time_compacting", "time_archived",
]
_UPdatable = [
    "title", "agent", "model", "cost", "tokens_input", "tokens_output",
    "tokens_reasoning", "tokens_cache_read", "tokens_cache_write", "metadata",
    "time_created", "time_updated",
]

def _upsert_session_sql():
    cols = ", ".join(_SESSION_COLS)
    ph = ", ".join(["?"] * len(_SESSION_COLS))
    sets = ", ".join(f"{c}=excluded.{c}" for c in _UPdatable)
    return (
        f"INSERT INTO session ({cols}) VALUES ({ph}) "
        f"ON CONFLICT(id) DO UPDATE SET {sets}"
    )

def _session_values(payload):
    sid = payload["session_id"]
    t = payload["totals"]
    return (
        sid, GLOBAL_PROJECT_ID, None, None,
        slugify(payload["title"], sid), "/", None,
        payload["title"], SYNC_VERSION,
        None, None, None, None, None,
        json.dumps(payload["metadata"], separators=(",", ":")),
        0, t["input"], t["output"], t["reasoning"],
        t["cache"]["read"], t["cache"]["write"],
        None, None, payload["agent"], json.dumps(payload["model_json"]),
        payload["created"], payload["updated"], None, None,
    )

INSERT_MESSAGE = (
    "INSERT INTO message (id, session_id, time_created, time_updated, data) "
    "VALUES (?,?,?,?,?)"
)
INSERT_PART = (
    "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) "
    "VALUES (?,?,?,?,?,?)"
)

def write_session(conn, payload):
    """Delete stale messages (cascades parts) then upsert session + insert rows.

    Returns the number of messages written.
    """
    sid = payload["session_id"]
    conn.execute("DELETE FROM message WHERE session_id = ?", (sid,))
    conn.execute(_upsert_session_sql(), _session_values(payload))
    n = 0
    for mp in payload["msg_payloads"]:
        conn.execute(INSERT_MESSAGE, (
            mp["msg_id"], sid, mp["time_created"], mp["time_created"],
            json.dumps(mp["msg_data"], separators=(",", ":")),
        ))
        conn.execute(INSERT_PART, (
            mp["part_id"], mp["msg_id"], sid, mp["time_created"], mp["time_created"],
            json.dumps(mp["part_data"], separators=(",", ":")),
        ))
        n += 1
    return n

def sync_changed(conn, payloads):
    """Single transaction: write all session payloads.

    Returns (sessions_written, messages_written).
    """
    s = m = 0
    conn.execute("BEGIN")
    try:
        for p in payloads:
            m += write_session(conn, p)
            s += 1
        conn.execute("COMMIT")
    except Exception:
        try:
            conn.execute("ROLLBACK")
        except sqlite3.Error:
            pass
        raise
    return s, m

def connect_target(db_path=OPENCODE_DB, read_only=False):
    mode = "ro" if read_only else "rw"
    conn = sqlite3.connect(f"file:{db_path}?mode={mode}", uri=True)
    for pragma in (
        "PRAGMA journal_mode=WAL",
        "PRAGMA busy_timeout=%d" % BUSY_TIMEOUT_MS,
        "PRAGMA foreign_keys=ON",
        "PRAGMA synchronous=NORMAL",
    ):
        try:
            conn.execute(pragma).fetchall()
        except sqlite3.Error:
            pass
    return conn

def collect_changes(encoder_dir, offset):
    """Return list of (db_path, session_dict) for changed sessions."""
    changes = []
    for db in find_encoder_dbs(encoder_dir):
        for sess in read_changed_sessions(db, offset):
            changes.append((db, sess))
    return changes

def run(args):
    verbose = args.verbose
    state = load_state(args.state_file)
    offset = state["last_offset"]
    if verbose:
        print(f"[sync] watermark offset={offset} "
              f"(last_sync={state.get('last_sync_at')})")

    changes = collect_changes(args.encoder_dir, offset)
    if not changes:
        print("[sync] Watermark up to date - nothing to do.")
        return 0

    payloads = []
    skipped = 0
    max_updated = offset
    for db, sess in changes:
        msgs = read_assistant_messages(db, sess["id"])
        max_updated = max(max_updated, int(sess.get("updated_at", 0) or 0))
        p = compute_session_payload(sess, msgs)
        if p is None:
            skipped += 1
            continue
        payloads.append(p)

    if verbose:
        print(f"[sync] changed sessions={len(changes)} "
              f"payloads={len(payloads)} skipped(no tokens)={skipped}")

    if args.dry_run:
        for p in payloads:
            t = p["totals"]
            print(f"  [dry-run] {p['session_id']} msgs={len(p['msg_payloads'])} "
                  f"in={t['input']} out={t['output']} cache_read={t['cache']['read']}")
        print(f"[sync] dry-run complete - no writes. new_offset would be {max_updated}")
        return 0

    conn = connect_target(args.opencode_db)
    try:
        s, m = sync_changed(conn, payloads)
    finally:
        conn.close()

    state["last_offset"] = max_updated
    state["last_sync_at"] = now_iso()
    state["total_sessions_synced"] = state.get("total_sessions_synced", 0) + s
    state["total_messages_synced"] = state.get("total_messages_synced", 0) + m
    save_state(args.state_file, state)

    print(f"[sync] synced sessions={s} messages={m} new_offset={max_updated}")
    return 0

def build_argparser():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--dry-run", action="store_true",
                    help="report only; write nothing and do not advance watermark")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--opencode-db", default=OPENCODE_DB)
    ap.add_argument("--encoder-dir", default=ENCODER_DIR)
    ap.add_argument("--state-file", default=STATE_FILE)
    return ap

def main(argv=None):
    args = build_argparser().parse_args(argv)
    try:
        return run(args)
    except sqlite3.Error as exc:
        print(f"[sync] ERROR: {exc}", file=sys.stderr)
        return 1

if __name__ == "__main__":
    sys.exit(main())
