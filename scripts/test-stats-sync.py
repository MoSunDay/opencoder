#!/usr/bin/env python3
"""Unit tests for opencoder-to-opencode-stats.py.

Self-contained: builds temp source (opencoder schema) and target (opencode
schema) SQLite DBs, then validates transforms, sync correctness, idempotency,
incremental watermarking, and the skip-zero-tokens rule. Plain functions, no
classes.
"""
import json
import os
import sqlite3
import sys
import tempfile

import importlib.util as _ilu
_HERE = os.path.dirname(os.path.abspath(__file__))
m = _ilu.module_from_spec(_ilu.spec_from_file_location(
    "opencoder_to_opencode_stats",
    os.path.join(_HERE, "opencoder-to-opencode-stats.py")))
m.__loader__.exec_module(m)

PASS = 0
FAIL = 0


def check(name, cond, detail=""):
    global PASS, FAIL
    if cond:
        PASS += 1
        print(f"  ok   {name}")
    else:
        FAIL += 1
        print(f"  FAIL {name}  {detail}")


# --- schema builders --------------------------------------------------------
SOURCE_SCHEMA = [
    "CREATE TABLE sessions ("
    " id TEXT PRIMARY KEY, title TEXT, agent TEXT, model TEXT,"
    " workdir_hash TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,"
    " summary TEXT, summary_seq INTEGER, handoff_seq INTEGER,"
    " handoff_plan TEXT, skill TEXT)",
    "CREATE TABLE messages ("
    " seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL,"
    " session_id TEXT NOT NULL, role TEXT NOT NULL, agent TEXT, model TEXT,"
    " blocks_json TEXT NOT NULL, usage_json TEXT NOT NULL,"
    " created_at INTEGER NOT NULL, synthetic INTEGER NOT NULL DEFAULT 0,"
    " mode TEXT, summary INTEGER NOT NULL DEFAULT 0)",
]


def make_target_schema():
    return [
        "CREATE TABLE project (id TEXT PRIMARY KEY, worktree TEXT)",
        "CREATE TABLE session ("
        " id TEXT PRIMARY KEY, project_id TEXT NOT NULL, workspace_id TEXT,"
        " parent_id TEXT, slug TEXT NOT NULL, directory TEXT NOT NULL, path TEXT,"
        " title TEXT NOT NULL, version TEXT NOT NULL, share_url TEXT,"
        " summary_additions INTEGER, summary_deletions INTEGER,"
        " summary_files INTEGER, summary_diffs TEXT, metadata TEXT,"
        " cost REAL DEFAULT 0 NOT NULL, tokens_input INTEGER DEFAULT 0 NOT NULL,"
        " tokens_output INTEGER DEFAULT 0 NOT NULL,"
        " tokens_reasoning INTEGER DEFAULT 0 NOT NULL,"
        " tokens_cache_read INTEGER DEFAULT 0 NOT NULL,"
        " tokens_cache_write INTEGER DEFAULT 0 NOT NULL, revert TEXT,"
        " permission TEXT, agent TEXT, model TEXT, time_created INTEGER NOT NULL,"
        " time_updated INTEGER NOT NULL, time_compacting INTEGER, time_archived INTEGER,"
        " FOREIGN KEY (project_id) REFERENCES project(id))",
        "CREATE TABLE message ("
        " id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER,"
        " time_updated INTEGER, data TEXT NOT NULL,"
        " FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE)",
        "CREATE TABLE part ("
        " id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL,"
        " time_created INTEGER, time_updated INTEGER, data TEXT NOT NULL,"
        " FOREIGN KEY (message_id) REFERENCES message(id) ON DELETE CASCADE)",
    ]


def make_target_db(path):
    conn = sqlite3.connect(path)
    conn.execute("PRAGMA foreign_keys=ON")
    for stmt in make_target_schema():
        conn.execute(stmt)
    conn.execute("INSERT INTO project (id, worktree) VALUES ('global', '/')")
    conn.commit()
    return conn


def make_source_db(path):
    conn = sqlite3.connect(path)
    for stmt in SOURCE_SCHEMA:
        conn.execute(stmt)
    conn.commit()
    return conn


def insert_session(conn, sid, title="t", agent="plan", model="glm-5.2/glm-5.2",
                   created=1000, updated=2000):
    conn.execute(
        "INSERT INTO sessions (id,title,agent,model,workdir_hash,"
        "created_at,updated_at,summary,summary_seq,handoff_seq,handoff_plan,skill)"
        " VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        (sid, title, agent, model, "h", created, updated, None, None, None, None, None),
    )


def insert_msg(conn, mid, sid, usage, created=1500, agent="plan", mode="plan"):
    conn.execute(
        "INSERT INTO messages (id,session_id,role,agent,model,blocks_json,"
        "usage_json,created_at,synthetic,mode,summary)"
        " VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        (mid, sid, "assistant", agent, "glm-5.2", "[]", json.dumps(usage),
         created, 0, mode, 0),
    )


# --- tests ------------------------------------------------------------------
def test_parse_usage():
    print("test_parse_usage")
    t = m.parse_usage('{"input_tokens":100,"output_tokens":20,'
                      '"total_tokens":120,"cache_read_tokens":80,'
                      '"cache_creation_tokens":5}')
    check("input", t["input"] == 100)
    check("output", t["output"] == 20)
    check("total", t["total"] == 120)
    check("cache_read", t["cache"]["read"] == 80)
    check("cache_write", t["cache"]["write"] == 5)
    # shape without cache fields
    t2 = m.parse_usage('{"input_tokens":3,"output_tokens":1,"total_tokens":4}')
    check("no-cache default 0", t2["cache"]["read"] == 0 and t2["cache"]["write"] == 0)
    check("reasoning default 0", t2["reasoning"] == 0)


def test_parse_model():
    print("test_parse_model")
    check("provider/model", m.parse_model("glm-5.2/glm-5.2") ==
          {"id": "glm-5.2", "providerID": "zhipuai-coding-plan", "variant": "default"})
    check("bare glm provider", m.parse_model("glm-5.2")["providerID"] == "zhipuai-coding-plan")
    check("bare model", m.parse_model("glm-5.2")["id"] == "glm-5.2")
    check("empty", m.parse_model("")["id"] == "unknown")
    check("variant default", m.parse_model("a/b")["variant"] == "default")


def test_sum_tokens():
    print("test_sum_tokens")
    agg = m.sum_tokens([
        m.parse_usage('{"input_tokens":10,"output_tokens":2,"total_tokens":12}'),
        m.parse_usage('{"input_tokens":5,"output_tokens":1,"total_tokens":6,'
                      '"cache_read_tokens":9}'),
    ])
    check("sum input", agg["input"] == 15)
    check("sum output", agg["output"] == 3)
    check("sum total", agg["total"] == 18)
    check("sum cache_read", agg["cache"]["read"] == 9)


def test_compute_payload_skip():
    print("test_compute_payload_skip")
    sess = {"id": "S1", "model": "glm-5.2/glm-5.2", "title": "hello",
            "created_at": 1000, "updated_at": 2000, "agent": "plan"}
    # only zero-token messages -> None
    msgs = [{"id": "M0", "usage_json": '{"input_tokens":0,"output_tokens":0,'
            '"total_tokens":0}', "created_at": 1500, "mode": "plan", "agent": "plan"}]
    check("skip all-zero", m.compute_session_payload(sess, msgs) is None)
    # no messages -> None
    check("skip empty", m.compute_session_payload(sess, []) is None)


def test_compute_payload_shape():
    print("test_compute_payload_shape")
    sess = {"id": "S1", "model": "glm-5.2/glm-5.2", "title": "hello world",
            "created_at": 1000, "updated_at": 2000, "agent": "plan"}
    msgs = [
        {"id": "M1", "usage_json": '{"input_tokens":10,"output_tokens":2,'
         '"total_tokens":12}', "created_at": 1500, "mode": "plan", "agent": "plan"},
        {"id": "M2", "usage_json": '{"input_tokens":5,"output_tokens":1,'
         '"total_tokens":6,"cache_read_tokens":9}', "created_at": 1600,
         "mode": "plan", "agent": "plan"},
    ]
    p = m.compute_session_payload(sess, msgs)
    check("target id", p["session_id"] == "enc_S1")
    check("totals input", p["totals"]["input"] == 15)
    check("totals cache_read", p["totals"]["cache"]["read"] == 9)
    check("two messages", len(p["msg_payloads"]) == 2)
    check("msg id prefixed", p["msg_payloads"][0]["msg_id"] == "enc_S1_M1")
    check("parent chain", p["msg_payloads"][1]["msg_data"]["parentID"] == "enc_S1_M1")
    check("first parent null", p["msg_payloads"][0]["msg_data"]["parentID"] is None)
    check("metadata source", p["metadata"]["source"] == "opencoder-sync")
    check("metadata orig id", p["metadata"]["opencoder_session_id"] == "S1")
    check("part type", p["msg_payloads"][0]["part_data"]["type"] == "step-finish")


def test_sync_basic():
    print("test_sync_basic")
    tmp = tempfile.mkdtemp()
    tgt = make_target_db(os.path.join(tmp, "tgt.db"))
    src_path = os.path.join(tmp, "src.db")
    src = make_source_db(src_path)
    insert_session(src, "S1", created=1000, updated=2000)
    insert_msg(src, "M1", "S1",
               {"input_tokens": 10, "output_tokens": 2, "total_tokens": 12},
               created=1500)
    insert_msg(src, "M2", "S1",
               {"input_tokens": 5, "output_tokens": 1, "total_tokens": 6,
                "cache_read_tokens": 9}, created=1600)
    src.commit()

    sess = m.read_changed_sessions(src_path, 0)[0]
    msgs = m.read_assistant_messages(src_path, "S1")
    p = m.compute_session_payload(sess, msgs)
    s, n = m.sync_changed(tgt, [p])

    check("wrote 1 session", s == 1)
    check("wrote 2 messages", n == 2)
    row = tgt.execute(
        "SELECT tokens_input, tokens_output, tokens_cache_read, model, agent,"
        " json_extract(metadata,'$.source') AS src,"
        " json_extract(metadata,'$.opencoder_session_id') AS oid"
        " FROM session WHERE id='enc_S1'"
    ).fetchone()
    check("session tokens_input", row[0] == 15, row)
    check("session tokens_output", row[1] == 3, row)
    check("session cache_read", row[2] == 9, row)
    check("model json", json.loads(row[3])["id"] == "glm-5.2")
    check("agent", row[4] == "plan")
    check("metadata source", row[5] == "opencoder-sync")
    check("metadata oid", row[6] == "S1")
    nmsg = tgt.execute("SELECT count(*) FROM message WHERE session_id='enc_S1'").fetchone()[0]
    npart = tgt.execute("SELECT count(*) FROM part WHERE session_id='enc_S1'").fetchone()[0]
    check("2 messages in db", nmsg == 2)
    check("2 parts in db", npart == 2)
    # session tokens == sum of message tokens (opencode invariant)
    tot = 0
    for (data,) in tgt.execute("SELECT data FROM message WHERE session_id='enc_S1'"):
        tot += json.loads(data)["tokens"]["input"]
    check("session==sum(message)", tot == row[0])
    tgt.close()
    src.close()


def test_sync_idempotent():
    print("test_sync_idempotent")
    tmp = tempfile.mkdtemp()
    tgt = make_target_db(os.path.join(tmp, "tgt.db"))
    src_path = os.path.join(tmp, "src.db")
    src = make_source_db(src_path)
    insert_session(src, "S1", created=1000, updated=2000)
    insert_msg(src, "M1", "S1",
               {"input_tokens": 10, "output_tokens": 2, "total_tokens": 12})
    src.commit()
    sess = m.read_changed_sessions(src_path, 0)[0]
    p = m.compute_session_payload(sess, m.read_assistant_messages(src_path, "S1"))

    m.sync_changed(tgt, [p])
    s2, n2 = m.sync_changed(tgt, [p])  # re-sync same payload
    nmsg = tgt.execute("SELECT count(*) FROM message WHERE session_id='enc_S1'").fetchone()[0]
    npart = tgt.execute("SELECT count(*) FROM part WHERE session_id='enc_S1'").fetchone()[0]
    check("re-sync still 1 session", s2 == 1)
    check("re-sync still 1 msg", n2 == 1)
    check("no dup messages", nmsg == 1, nmsg)
    check("no dup parts", npart == 1, npart)
    # one extra message would be a 4th part; ensure parts == msgs
    tgt.close()
    src.close()


def test_incremental_watermark():
    print("test_incremental_watermark")
    tmp = tempfile.mkdtemp()
    src_path = os.path.join(tmp, "src.db")
    src = make_source_db(src_path)
    insert_session(src, "OLD", created=1000, updated=1000)
    insert_msg(src, "M_OLD", "OLD",
               {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2})
    insert_session(src, "NEW", created=3000, updated=3000)
    insert_msg(src, "M_NEW", "NEW",
               {"input_tokens": 7, "output_tokens": 3, "total_tokens": 10})
    src.commit()
    # offset 2000 -> only NEW
    got = m.read_changed_sessions(src_path, 2000)
    check("only newer session", len(got) == 1 and got[0]["id"] == "NEW", got)
    # offset 0 -> both
    check("offset 0 all", len(m.read_changed_sessions(src_path, 0)) == 2)
    src.close()


def test_cross_session_same_msg_id():
    """Two sessions sharing a source message id must produce distinct rows."""
    print("test_cross_session_same_msg_id")
    tmp = tempfile.mkdtemp()
    tgt = make_target_db(os.path.join(tmp, "tgt.db"))
    src_path = os.path.join(tmp, "src.db")
    src = make_source_db(src_path)
    for sid in ("SA", "SB"):
        insert_session(src, sid, created=1000, updated=2000)
        insert_msg(src, "SHARED", sid,
                   {"input_tokens": 4, "output_tokens": 1, "total_tokens": 5})
    src.commit()
    payloads = []
    for sid in ("SA", "SB"):
        sess = [x for x in m.read_changed_sessions(src_path, 0) if x["id"] == sid][0]
        payloads.append(m.compute_session_payload(sess, m.read_assistant_messages(src_path, sid)))
    s, n = m.sync_changed(tgt, [payloads[0], payloads[1]])
    check("both sessions", s == 2)
    check("both messages", n == 2)
    nmsg = tgt.execute("SELECT count(*) FROM message").fetchone()[0]
    npart = tgt.execute("SELECT count(*) FROM part").fetchone()[0]
    check("distinct msg rows", nmsg == 2, nmsg)
    check("distinct part rows", npart == 2, npart)
    tgt.close(); src.close()


def main():
    test_parse_usage()
    test_parse_model()
    test_sum_tokens()
    test_compute_payload_skip()
    test_compute_payload_shape()
    test_sync_basic()
    test_sync_idempotent()
    test_incremental_watermark()
    test_cross_session_same_msg_id()
    print(f"\n{PASS} passed, {FAIL} failed")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
