"""Todos-workflow e2e scenarios against real glm5.2 (E19b/E19c).

Deepens the E19 todos smoke in cli_scenarios.py: multi-TODO DAG execution,
the `--debug` filesystem projection, append-only event cursor semantics, and
cross-process interrupt -> resume recovery with its exit-code contract.

Contract depth map (HARD = deterministic store/contract assert;
SOFT = model-cooperation-dependent, recorded as skip not failure):
  E19b    multi-TODO DAG + --debug projection + events cursor
          HARD  todos run --debug rc==0; stdout is pure state JSON;
                stderr carries workflow_id=; debug projection: exactly one
                <data_local>/opencoder/<hash>/todos/<wf_id> dir holding
                task-info/{workflow,index}.json, task-info/todos/{t1,t2}.json,
                process/workflow/events.ndjson, sessions/parent.json,
                sessions/todos/t1/attempt-001.json; events: rc==0 non-empty,
                seq strictly increasing, causal order
                todo_accepted(t1) < todos_dispatched(t2); --after cursor
                (SQL `seq > after`): after=last -> empty, after=last-1 ->
                exactly one event with seq == last
          SOFT  final status completed; t1+t2 passed; dag_done.txt on disk
  E19c    interrupt -> resume cross-process recovery + exit-code contract
          HARD  background run exposes workflow_id= on stderr; a second
                process observes todos_dispatched while t1 is in flight;
                todos interrupt rc==0 and reports suspended; the interrupted
                run exits rc==1 with the suspended state JSON on stdout
                (non-local suspension maps to the error exit, not 130);
                todos resume rc==0 with state JSON
          SOFT  resumed workflow completes; int_done.txt artifact exists

Run standalone:  python3 scripts/e2e/todos_scenarios.py [binary]
(requires ZHIPU_API_KEY or an installed auth.json — the todos drive real
model sessions, unlike the key-free config_scenarios suite).
"""

from __future__ import annotations

import glob
import json
import os
import re
import subprocess
import sys
import tempfile
import time

try:
    from . import lib
    from .lib import Counter
except ImportError:  # standalone: python3 scripts/e2e/todos_scenarios.py
    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    from e2e import lib
    from e2e.lib import Counter


def _run_split(bin_path: str, args: list[str], timeout: int = 900) -> tuple[int, str, str]:
    """Run the binary with stdout/stderr captured SEPARATELY (lib.run merges)."""
    try:
        p = subprocess.run([bin_path] + args, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout or "", p.stderr or ""
    except subprocess.TimeoutExpired:
        return 124, "", f"TIMEOUT after {timeout}s"


def _json_or_none(text: str):
    try:
        return json.loads(text)
    except Exception:
        return None


def _dispatch_has_todo(payload, todo_id: str) -> bool:
    """todos_dispatched payload {"todos":[{todo_id,...}],...} mentions todo_id."""
    if not isinstance(payload, dict):
        return False
    todos = payload.get("todos")
    return isinstance(todos, list) and any(
        isinstance(d, dict) and d.get("todo_id") == todo_id for d in todos
    )


def _e19b_dag(c: Counter, bin_path: str, api_key: str) -> None:
    print("== E19b: multi-TODO DAG + --debug projection + events cursor ==")
    cfg = lib.make_config(api_key=api_key)
    wd = lib.seed_workdir(cfg)
    cat_gate = {
        "name": "bash",  # bash tool input field is "command" (tools/bash.rs);
        # the Rust gate is json_contains: expected must be a recursive subset
        # of the actual tool input, so a plain cat must be run verbatim.
        "arguments_contains": {"command": "cat dag_done.txt"},
        "result_ok": True,
    }
    spec = {
        "schema_version": 1, "id": "todos-dag", "name": "todos dag",
        "objective": "Create dag_done.txt containing 'ok' and verify it with cat.",
        "constraints": [], "metadata": {},
        "todos": [
            {
                "id": "t1", "title": "create marker file",
                "requirement_background": "e2e dag marker todo",
                "instructions": (
                    "Use the write tool to create a file named dag_done.txt in the current "
                    "directory with the exact single-line content 'ok'. Then run "
                    "'cat dag_done.txt' with the bash tool to verify it prints ok."
                ),
                "depends_on": [], "agent": "act", "max_attempts": 3,
                "acceptance": {"criteria": "dag_done.txt exists and cat prints ok",
                               "required_tool_calls": [dict(cat_gate)]},
            },
            {
                "id": "t2", "title": "verify marker file",
                "requirement_background": "e2e dag verification todo",
                "instructions": "Run 'cat dag_done.txt' with the bash tool and confirm "
                                "it prints ok.",
                "depends_on": ["t1"], "agent": "act", "max_attempts": 3,
                "acceptance": {"criteria": "cat dag_done.txt prints ok",
                               "required_tool_calls": [dict(cat_gate)]},
            },
        ],
    }
    spec_path = os.path.join(wd, "todos-dag.json")
    lib.write_file(wd, "todos-dag.json", json.dumps(spec))

    rc, out, err = _run_split(bin_path, ["--workdir", wd, "todos", "run",
                                         "--file", spec_path, "--debug", "--json"])
    c.check("E19b todos run --debug rc==0", rc == 0, f"rc={rc} err_tail={err[-300:]}")
    state = _json_or_none(out)
    c.check("E19b todos run stdout is pure state JSON (dict)", isinstance(state, dict),
            f"stdout_tail={out[-200:]}")
    c.check("E19b todos run stderr carries workflow_id=", "workflow_id=" in err,
            f"err_tail={err[-200:]}")
    m = re.search(r"workflow_id=(\S+)", err)
    wf_id = m.group(1) if m else None
    c.check("E19b workflow_id extractable from stderr", bool(wf_id),
            f"err_tail={err[-200:]}")

    # --debug projection: <data_local>/opencoder/<DefaultHasher-hash>/todos/<wf>.
    # The hash cannot be recomputed in Python, so glob for the wf_id instead.
    if wf_id:
        data_local = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
        matches = glob.glob(os.path.join(data_local, "opencoder", "*", "todos", wf_id))
        c.check("E19b debug projection dir exists exactly once (globbed)",
                len(matches) == 1 and os.path.isdir(matches[0]), f"matches={matches}")
        if matches:
            for rel in (
                "task-info/workflow.json",
                "task-info/index.json",
                "task-info/todos/t1.json",
                "task-info/todos/t2.json",
                "process/workflow/events.ndjson",
                "sessions/parent.json",
                "sessions/todos/t1/attempt-001.json",
            ):
                c.check(f"E19b debug file exists: {rel}",
                        os.path.isfile(os.path.join(matches[0], rel)))
    else:
        c.check("E19b debug projection dir exists exactly once (globbed)", False,
                "no workflow_id — run failed")

    # Append-only event log: ordering + strictly-greater --after cursor.
    if wf_id:
        rc_e, out_e, err_e = _run_split(bin_path, ["--workdir", wd, "todos", "events",
                                                   wf_id, "--json"], 60)
        events = _json_or_none(out_e)
        c.check("E19b todos events rc==0 json list non-empty",
                rc_e == 0 and isinstance(events, list) and len(events) > 0,
                f"rc={rc_e} err_tail={err_e[-160:]}")
        if isinstance(events, list) and events:
            seqs = [e.get("seq") for e in events if isinstance(e, dict)]
            c.check("E19b every event has an integer seq",
                    len(seqs) == len(events) and all(isinstance(s, int) for s in seqs),
                    f"seqs={seqs}")
            if len(seqs) == len(events) and seqs:
                c.check("E19b event seq strictly increasing",
                        all(a < b for a, b in zip(seqs, seqs[1:])), f"seqs={seqs}")
                idx_accept = next(
                    (i for i, e in enumerate(events)
                     if e.get("kind") == "todo_accepted"
                     and (e.get("payload") or {}).get("todo_id") == "t1"), None)
                idx_dispatch_t2 = next(
                    (i for i, e in enumerate(events)
                     if e.get("kind") == "todos_dispatched"
                     and _dispatch_has_todo(e.get("payload"), "t2")), None)
                c.check("E19b causal order: todo_accepted(t1) < todos_dispatched(t2)",
                        idx_accept is not None and idx_dispatch_t2 is not None
                        and idx_accept < idx_dispatch_t2,
                        f"idx_accept={idx_accept} idx_dispatch_t2={idx_dispatch_t2}")
                # Cursor: SQL filter is `seq > after`.
                last = max(seqs)
                rc_a, out_a, _ = _run_split(
                    bin_path, ["--workdir", wd, "todos", "events", wf_id,
                               "--json", "--after", str(last)], 60)
                ev_a = _json_or_none(out_a)
                c.check("E19b events --after last yields empty list",
                        rc_a == 0 and ev_a == [], f"rc={rc_a} out_tail={out_a[-120:]}")
                rc_b, out_b, _ = _run_split(
                    bin_path, ["--workdir", wd, "todos", "events", wf_id,
                               "--json", "--after", str(last - 1)], 60)
                ev_b = _json_or_none(out_b)
                c.check("E19b events --after last-1 yields exactly the last event",
                        rc_b == 0 and isinstance(ev_b, list) and len(ev_b) == 1
                        and ev_b[0].get("seq") == last,
                        f"rc={rc_b} n={len(ev_b) if isinstance(ev_b, list) else None}")
    else:
        c.check("E19b todos events rc==0 json list non-empty", False,
                "no workflow_id — run failed")

    # Model-cooperation soft checks: DAG outcome + artifact.
    if isinstance(state, dict):
        c.soft("E19b final workflow status completed", state.get("status") == "completed",
               f"status={state.get('status')}")
        todos_state = state.get("todos") or {}
        c.soft("E19b todo t1 passed",
               (todos_state.get("t1") or {}).get("status") == "passed",
               "t1 not passed (model/acceptance did not cooperate)")
        c.soft("E19b todo t2 passed",
               (todos_state.get("t2") or {}).get("status") == "passed",
               "t2 not passed (model/acceptance did not cooperate)")
    c.soft("E19b dag_done.txt artifact exists", os.path.isfile(os.path.join(wd, "dag_done.txt")),
           "file missing (model did not finish the write)")


def _e19c_interrupt_resume(c: Counter, bin_path: str, api_key: str) -> None:
    print("== E19c: interrupt -> resume cross-process recovery + exit-code contract ==")
    cfg = lib.make_config(api_key=api_key)
    wd = lib.seed_workdir(cfg)
    spec = {
        "schema_version": 1, "id": "todos-interrupt", "name": "todos interrupt",
        "objective": "Sleep first, then create int_done.txt containing 'ok'.",
        "constraints": [], "metadata": {},
        "todos": [{
            "id": "t1", "title": "slow marker todo",
            "requirement_background": "e2e interrupt/resume todo",
            "instructions": (
                "First run 'sleep 45' with the bash tool and wait for it to finish. "
                "Then create int_done.txt containing 'ok' and return the final candidate JSON."
            ),
            "depends_on": [], "agent": "act", "max_attempts": 3,
            "acceptance": {"criteria": "int_done.txt exists containing ok",
                           "required_tool_calls": []},
        }],
    }
    spec_path = os.path.join(wd, "todos-interrupt.json")
    lib.write_file(wd, "todos-interrupt.json", json.dumps(spec))

    # Background run: stdout/stderr go to SEPARATE temp files (never pipes —
    # the run process tails its own progress to stderr; a pipe we never drain
    # would deadlock it). The temp files share one file offset with the child,
    # so reading to EOF and tracking `pos` keeps parent/child in lockstep.
    out_f = tempfile.TemporaryFile(mode="w+")
    err_f = tempfile.TemporaryFile(mode="w+")
    p = subprocess.Popen([bin_path, "--workdir", wd, "todos", "run",
                          "--file", spec_path, "--json"],
                         stdout=out_f, stderr=err_f)
    try:
        wf_id = None
        pos = 0
        err_text = ""
        wf_deadline = time.time() + 120
        while time.time() < wf_deadline:
            err_f.seek(pos)
            chunk = err_f.read()
            if chunk:
                pos = err_f.tell()
                err_text += chunk
            m = re.search(r"workflow_id=(\S+)", err_text)
            if m:
                wf_id = m.group(1)
                break
            if p.poll() is not None:
                break  # process gone: one final drain below, then give up
            time.sleep(1.0)
        err_f.seek(pos)
        err_text += err_f.read()
        if wf_id is None:
            m = re.search(r"workflow_id=(\S+)", err_text)
            wf_id = m.group(1) if m else None
        c.check("E19c workflow_id observed on background run stderr", bool(wf_id),
                f"err_tail={err_text[-200:]}")

        # Second process must see todos_dispatched while t1 is mid-flight
        # (the sleep 45 keeps the turn open).
        dispatched = False
        if wf_id:
            dl = time.time() + 240
            while time.time() < dl:
                rc_q, out_q, _ = _run_split(bin_path, ["--workdir", wd, "todos", "events",
                                                       wf_id, "--json"], 60)
                evs = _json_or_none(out_q)
                if isinstance(evs, list) and any(e.get("kind") == "todos_dispatched"
                                                 for e in evs if isinstance(e, dict)):
                    dispatched = True
                    break
                if p.poll() is not None:
                    break  # run exited: it can never dispatch again
                time.sleep(1.0)
        c.check("E19c todos_dispatched observed from a second process (t1 in flight)",
                dispatched,
                "no workflow_id" if not wf_id
                else "dispatch not observed before run ended/deadline")

        if wf_id:
            rc_i, out_i, err_i = _run_split(
                bin_path, ["--workdir", wd, "todos", "interrupt", wf_id], 60)
            c.check("E19c todos interrupt rc==0 and reports suspended",
                    rc_i == 0 and "suspended" in out_i,
                    f"rc={rc_i} out={out_i.strip()[:120]} err_tail={err_i[-160:]}")
        else:
            c.check("E19c todos interrupt rc==0 and reports suspended", False,
                    "no workflow_id — background run failed")

        # The interrupted run must terminate on its own (poll_interrupt notices
        # the store generation change within ~250ms) and map the non-local
        # suspension to the error exit code 1 (not the local-Ctrl-C 130).
        exited = True
        try:
            p.wait(timeout=300)
        except subprocess.TimeoutExpired:
            exited = False
            p.kill()
            p.wait()
        if exited:
            c.check("E19c interrupted run exit code is 1 (suspended, non-local)",
                    p.returncode == 1, f"rc={p.returncode}")
        else:
            c.check("E19c interrupted run exits within 300s", False,
                    "still alive after interrupt; killed")
        out_f.seek(0)
        run_out = out_f.read()
        run_state = _json_or_none(run_out)
        c.check("E19c run stdout is the suspended state JSON",
                isinstance(run_state, dict) and run_state.get("status") == "suspended",
                f"stdout_tail={run_out[-200:]}")

        # Resume in yet another process: the suspended generation is taken
        # over and driven to a terminal state.
        if wf_id and isinstance(run_state, dict) and run_state.get("status") == "suspended":
            rc_r, out_r, err_r = _run_split(
                bin_path, ["--workdir", wd, "todos", "resume", wf_id, "--json"], 900)
            res = _json_or_none(out_r)
            c.check("E19c todos resume rc==0 with state JSON",
                    rc_r == 0 and isinstance(res, dict),
                    f"rc={rc_r} err_tail={err_r[-200:]}")
            c.soft("E19c resumed workflow completes",
                   isinstance(res, dict) and res.get("status") == "completed",
                   "resume did not reach completed (model/acceptance did not cooperate)")
        else:
            c.check("E19c todos resume rc==0 with state JSON", False,
                    "no suspended state to resume (earlier step failed)")
        c.soft("E19c int_done.txt artifact exists",
               os.path.isfile(os.path.join(wd, "int_done.txt")),
               "file missing (model did not finish the write)")
    finally:
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait()
        out_f.close()
        err_f.close()


def run_all(bin_path: str, api_key: str) -> Counter:
    """Run every todos-workflow scenario. NOTE: drives real model sessions —
    the todos execute live agents, so this needs the API key."""
    c = Counter()
    _e19b_dag(c, bin_path, api_key)
    _e19c_interrupt_resume(c, bin_path, api_key)
    c.summary("Todos scenarios")
    return c


def _main() -> int:
    import argparse

    ap = argparse.ArgumentParser(
        description="opencoder todos-workflow e2e (E19b/E19c; drives a real model)"
    )
    ap.add_argument("binary", nargs="?", default=None, help="path to the opencoder binary")
    args = ap.parse_args()

    bin_path = lib.resolve_bin(args.binary)
    if not os.path.isfile(bin_path):
        print(f"FAIL: binary not found: {bin_path}", file=sys.stderr)
        return 2
    api_key = lib.ensure_auth()
    total = run_all(bin_path, api_key)
    print("\n" + "=" * 60)
    print(f"todos e2e result: {total.passed} passed, {total.failed} failed, "
          f"{total.skipped} skipped")
    print("=" * 60)
    return 1 if total.failed else 0


if __name__ == "__main__":
    sys.exit(_main())
