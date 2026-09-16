"""Todos-runtime e2e scenarios against real glm5.2 (E22).

Complements E19b/E19c (todos_scenarios.py) with the contracts that suite
does not cover. HARD = deterministic contract; SOFT = model cooperation.
  E22a  success main flow: run --json output contracts (pure state doc on
        stdout, workflow_id= on stderr, exit 0), the {seq,workflow_id,kind,
        payload,ts} event shape with the full happy-path event catalog, the
        --after cursor suffix, no projection without --debug, the `show`
        one-line banner and pretty-run == compact-resume on resume.
  E22b  deterministic failure (max_attempts=1 + an unsatisfiable
        required_tool_calls gate): exit 1 with a failed state carrying
        terminal_reason, todo_failed + workflow_failed events, no
        attempt-002 session, idempotent resume (exit 1, zero new events),
        interrupt refused on the terminal workflow.
  E22c  local Ctrl-C + --debug refresh: resume-on-Running refusal, SIGINT ->
        rc 130 + suspended state ("local interrupt requested"), projection
        refreshed (workflow_interrupted + suspended index), then resume
        --debug completing with workflow_resumed and index.json in sync.
  E22d  directory-format spec + env.json binding to agent.share_dir:
        env_vars ride into the todo bash step (bound value lands in a file).

Standalone:  python3 scripts/e2e/todos_runtime_scenarios.py [binary]
Needs ZHIPU_API_KEY or an installed auth.json (the todos drive real model
sessions; the key-free contracts live in todos_contract_scenarios.py).
"""

from __future__ import annotations

import glob
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time

try:
    from . import lib
    from . import todos_contract_scenarios as tc
    from .lib import Counter, json_or_none, run_split
except ImportError:  # standalone: python3 scripts/e2e/todos_runtime_scenarios.py
    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    from e2e import lib
    from e2e import todos_contract_scenarios as tc
    from e2e.lib import Counter, json_or_none, run_split

R1_EVENT_KINDS = ("workflow_created", "workflow_started", "todos_dispatched",
                  "todo_candidate_ready", "todo_acceptance_started",
                  "todo_accepted", "workflow_completed")
EVENT_SHAPE = {"seq", "workflow_id", "kind", "payload", "ts"}


def _gate(cmd: str) -> dict:
    # json_contains: expected must be a recursive subset of the tool input.
    return {"name": "bash", "arguments_contains": {"command": cmd}, "result_ok": True}


def _cat_spec(wd: str, name: str, artifact: str, *, max_attempts=3) -> str:
    """Legacy single-file spec: write <artifact> then verify with `cat`."""
    spec = tc.legacy_spec(name, [tc.todo("t1",
        instructions=(f"Create {artifact} containing 'ok', then run "
                      f"'cat {artifact}' with the bash tool to verify it."),
        max_attempts=max_attempts,
        acceptance={"criteria": f"{artifact} exists and cat prints ok",
                    "required_tool_calls": [_gate(f"cat {artifact}")]},
    )])
    path = os.path.join(wd, f"{name}.json")
    lib.write_file(wd, f"{name}.json", json.dumps(spec))
    return path


def _projection(wf_id: str) -> str | None:
    data_local = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    matches = glob.glob(os.path.join(data_local, "opencoder", "*", "todos", wf_id))
    return matches[0] if len(matches) == 1 else None


def _proj_state(proj: str):
    """(events.ndjson text, index.json dict) of a --debug projection."""
    nd = open(os.path.join(proj, "process/workflow/events.ndjson")).read()
    idx = json_or_none(open(os.path.join(proj, "task-info/index.json")).read())
    return nd, idx


def _events(bin_path: str, wd: str, wf_id: str | None, after: int | None = None):
    args = ["--workdir", wd, "todos", "events", wf_id or "none", "--json"]
    if after is not None:
        args += ["--after", str(after)]
    rc, out, err = run_split(bin_path, args, 120)
    return rc, json_or_none(out), err


def _e22a_success(c: Counter, bin_path: str, api_key: str) -> None:
    print("== E22a: success main flow — output contracts + event catalog ==")
    wd = lib.seed_workdir(lib.make_config(api_key=api_key))
    spec_path = _cat_spec(wd, "e22a", "e22a_done.txt")
    rc, out, err = run_split(bin_path, ["--workdir", wd, "todos", "run",
                                        "--file", spec_path, "--json"])
    c.check("E22a todos run --json rc==0", rc == 0, f"rc={rc} err_tail={err[-300:]}")
    state = json_or_none(out)
    c.check("E22a run stdout is one pure JSON document", isinstance(state, dict),
            f"stdout_tail={out[-200:]}")
    wf_id = m.group(1) if (m := re.search(r"workflow_id=(\S+)", err)) else None
    c.check("E22a stderr carries workflow_id=", bool(wf_id), f"err_tail={err[-200:]}")

    rc_e, events, err_e = _events(bin_path, wd, wf_id)
    c.check("E22a events rc==0 non-empty list", rc_e == 0 and isinstance(events, list)
            and len(events) > 0, f"rc={rc_e} err_tail={err_e[-160:]}")
    for kind in R1_EVENT_KINDS:
        c.check(f"E22a event kind present: {kind}",
                any(isinstance(e, dict) and e.get("kind") == kind for e in events or []))
    c.check("E22a events --json element shape {seq,workflow_id,kind,payload,ts}",
            all(isinstance(e, dict) and set(e) == EVENT_SHAPE for e in events or []),
            f"first={events[0] if events else None}")

    if isinstance(events, list) and events:
        seqs = [e["seq"] for e in events if isinstance(e.get("seq"), int)]
        mid = seqs[len(seqs) // 2]
        rc_m, tail, _ = _events(bin_path, wd, wf_id, after=mid)
        expected = [e["seq"] for e in events
                    if isinstance(e.get("seq"), int) and e["seq"] > mid]
        c.check("E22a --after mid cursor returns the exact continuous suffix",
                rc_m == 0 and [e.get("seq") for e in tail or []] == expected,
                f"mid={mid} got={[e.get('seq') for e in tail or []]}")

    data_local = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    c.check("E22a without --debug no projection dir appears",
            not glob.glob(os.path.join(data_local, "opencoder", "*", "todos",
                                       wf_id or "no-id")))

    if wf_id:
        rc_s, out_s, _ = run_split(bin_path, ["--workdir", wd, "todos", "show",
                                              wf_id], 60)
        first = (out_s.splitlines() or [""])[0]
        c.check("E22a show text first line '<name> [<status>] generation=N'",
                rc_s == 0 and re.fullmatch(r"e22a \[\w+\] generation=\d+", first),
                f"first_line={first!r}")
        rc_r, out_r, err_r = run_split(bin_path, ["--workdir", wd, "todos", "resume",
                                                  wf_id, "--json"], 120)
        res = json_or_none(out_r)
        c.check("E22a resume on completed: exit 0 + compact state JSON",
                rc_r == 0 and isinstance(res, dict), f"rc={rc_r} out={out_r[:120]}")
        c.check("E22a pretty run doc == compact resume doc",
                isinstance(res, dict) and res == state, "documents differ")
    c.soft("E22a final status completed", isinstance(state, dict)
           and state.get("status") == "completed",
           "model/acceptance did not cooperate")
    c.soft("E22a t1 passed", isinstance(state, dict)
           and ((state.get("todos") or {}).get("t1") or {}).get("status") == "passed",
           "t1 not passed")
    c.soft("E22a artifact exists", os.path.isfile(os.path.join(wd, "e22a_done.txt")),
           "file missing")


def _e22b_failure(c: Counter, bin_path: str, api_key: str) -> None:
    print("== E22b: max_attempts exhaustion -> failed -> idempotent resume ==")
    wd = lib.seed_workdir(lib.make_config(api_key=api_key))
    forbidden = "e2e-must-never-run --version"
    spec = tc.legacy_spec("e22b", [tc.todo("t1",
        instructions=("Create e22b_note.txt containing 'ok'. NEVER run the command "
                      f"'{forbidden}' — it is forbidden. Do not run it, then "
                      "return the final candidate JSON."),
        max_attempts=1,
        acceptance={"criteria": "gate demands a forbidden tool call",
                    "required_tool_calls": [_gate(forbidden)]},
    )])
    spec_path = os.path.join(wd, "e22b.json")
    lib.write_file(wd, "e22b.json", json.dumps(spec))

    rc, out, err = run_split(bin_path, ["--workdir", wd, "todos", "run",
                                        "--file", spec_path, "--debug", "--json"])
    c.check("E22b doomed todos run exits 1", rc == 1, f"rc={rc} err_tail={err[-240:]}")
    state = json_or_none(out)
    c.check("E22b run stdout is the failed state JSON with terminal_reason",
            isinstance(state, dict) and state.get("status") == "failed"
            and bool(state.get("terminal_reason")),
            f"stdout_tail={out[-200:]}")
    wf_id = (state or {}).get("workflow_id")
    _, events, _ = _events(bin_path, wd, wf_id)
    kinds = [e.get("kind") for e in events or []]
    c.check("E22b events contain todo_failed", "todo_failed" in kinds, f"kinds={kinds}")
    c.check("E22b events contain workflow_failed", "workflow_failed" in kinds,
            f"kinds={kinds}")

    if wf_id:
        proj = _projection(wf_id)
        c.check("E22b --debug projection exists", bool(proj), f"proj={proj}")
        if proj:
            attempts = os.listdir(os.path.join(proj, "sessions", "todos", "t1"))
            c.check("E22b no attempt-002 session (attempts exhausted at 1)",
                    attempts == ["attempt-001.json"], f"attempts={attempts}")
        rc_r, out_r, err_r = run_split(bin_path, ["--workdir", wd, "todos", "resume",
                                                  wf_id, "--json"], 120)
        res = json_or_none(out_r)
        c.check("E22b resume on failed: state re-printed, exit 1 (terminal error)",
                rc_r == 1 and isinstance(res, dict)
                and res.get("status") == "failed"
                and res.get("workflow_id") == wf_id,
                f"rc={rc_r} out={out_r[:120]} err={err_r[-160:]}")
        _, events_after, _ = _events(bin_path, wd, wf_id)
        c.check("E22b resume on failed is idempotent (zero new events)",
                isinstance(events_after, list) and len(events_after) == len(events or []),
                f"before={len(events or [])} after={len(events_after or [])}")
        rc_i, out_i, err_i = run_split(bin_path, ["--workdir", wd, "todos",
                                                  "interrupt", wf_id], 60)
        c.check("E22b interrupt on terminal workflow refused",
                rc_i == 1 and "cannot interrupt terminal workflow" in err_i,
                f"rc={rc_i} err={err_i[-200:]}")
    c.soft("E22b todo t1 failed", isinstance(state, dict)
           and ((state.get("todos") or {}).get("t1") or {}).get("status") == "failed",
           "todo did not fail")


def _e22c_interrupt_resume(c: Counter, bin_path: str, api_key: str) -> None:
    print("== E22c: local Ctrl-C 130 + --debug projection refresh ==")
    wd = lib.seed_workdir(lib.make_config(api_key=api_key))
    spec = tc.legacy_spec("e22c", [tc.todo("t1",
        instructions=("First run 'sleep 45' with the bash tool and wait for it to "
                      "finish. Then create e22c_done.txt containing 'ok'."),
        acceptance={"criteria": "e22c_done.txt exists containing ok",
                    "required_tool_calls": []},
    )])
    spec_path = os.path.join(wd, "e22c.json")
    lib.write_file(wd, "e22c.json", json.dumps(spec))

    # Separate temp files for the child (a never-drained pipe would deadlock).
    out_f = tempfile.TemporaryFile(mode="w+")
    err_f = tempfile.TemporaryFile(mode="w+")
    p = subprocess.Popen([bin_path, "--workdir", wd, "todos", "run",
                          "--file", spec_path, "--debug", "--json"],
                         stdout=out_f, stderr=err_f)
    try:
        wf_id, pos, err_text = None, 0, ""
        deadline = time.time() + 120
        while time.time() < deadline:
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
                break
            time.sleep(1.0)
        c.check("E22c workflow_id observed on background run stderr", bool(wf_id),
                f"err_tail={err_text[-200:]}")

        dispatched = False
        if wf_id:
            dl = time.time() + 240
            while time.time() < dl:
                _, evs, _ = _events(bin_path, wd, wf_id)
                if any(e.get("kind") == "todos_dispatched" for e in evs or []):
                    dispatched = True
                    break
                if p.poll() is not None:
                    break
                time.sleep(1.0)
        c.check("E22c todos_dispatched observed while t1 in flight", dispatched)

        if wf_id and p.poll() is None:
            rc_r, out_r, err_r = run_split(bin_path, ["--workdir", wd, "todos",
                                                      "resume", wf_id, "--json"], 60)
            c.check("E22c resume on Running refused (interrupt first)",
                    rc_r == 1 and "still running" in err_r and "interrupt" in err_r,
                    f"rc={rc_r} err={err_r[-220:]}")

        p.send_signal(signal.SIGINT)
        exited = True
        try:
            p.wait(timeout=120)
        except subprocess.TimeoutExpired:
            exited = False
            p.kill()
            p.wait()
        c.check("E22c local SIGINT exits rc==130", exited and p.returncode == 130,
                f"rc={p.returncode if exited else 'alive'}")
        out_f.seek(0)
        state = json_or_none(out_f.read())
        c.check("E22c stdout is suspended state with 'local interrupt requested'",
                isinstance(state, dict) and state.get("status") == "suspended"
                and state.get("terminal_reason") == "local interrupt requested",
                f"stdout_tail={out_f.read()[-200:]}")

        if wf_id:
            proj = _projection(wf_id)
            c.check("E22c --debug projection exists", bool(proj))
            if proj:
                nd, idx = _proj_state(proj)
                c.check("E22c events.ndjson refreshed with workflow_interrupted",
                        "workflow_interrupted" in nd)
                c.check("E22c index.json says suspended",
                        isinstance(idx, dict) and idx.get("status") == "suspended",
                        f"status={(idx or {}).get('status')}")
            rc_res, out_res, err_res = run_split(
                bin_path, ["--workdir", wd, "todos", "resume", wf_id,
                           "--debug", "--json"], 900)
            res = json_or_none(out_res)
            c.check("E22c todos resume --debug rc==0", rc_res == 0,
                    f"rc={rc_res} err={err_res[-200:]}")
            if proj:
                nd, idx = _proj_state(proj)
                c.check("E22c events.ndjson refreshed with workflow_resumed",
                        "workflow_resumed" in nd)
                c.check("E22c index.json matches the final state",
                        isinstance(idx, dict) and isinstance(res, dict)
                        and idx.get("status") == res.get("status"),
                        f"idx={(idx or {}).get('status')} res={res}")
            c.soft("E22c resumed workflow completes", isinstance(res, dict)
                   and res.get("status") == "completed",
                   "resume did not reach completed")
        c.soft("E22c artifact exists", os.path.isfile(os.path.join(wd, "e22c_done.txt")),
               "file missing")
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


def _e22d_dir_format_env(c: Counter, bin_path: str, api_key: str, workdir: str) -> None:
    print("== E22d: directory-format spec + env.json binding (env passthrough) ==")
    wd = lib.seed_workdir(lib.make_config(api_key=api_key))
    env = "e2e-runtime-env"
    share = os.path.join(workdir, "share-runtime")
    lib.write_file(share, "env", "")  # ensure root
    ctx = {"context": f"Env {env}: a demo environment for e2e.",
           "env_vars": {"E2E_MARKER": "runtime-v1"}}
    lib.write_file(share, f"env/{env}/context.json", json.dumps(ctx))
    manifest = {
        "version": 1,
        "name": "e22d",
        "todos": ["t1"],
        "defaults": {"max_attempts": 3},
    }
    t1 = {
        "id": "t1",
        "title": "marker",
        "goal": "verify env passthrough",
        "max_attempts": 3,
        "tools": ["bash"],
        "context_files": [],
        "acceptance": {"criteria": "marker file contains runtime-v1",
                       "required_tool_calls": []},
    }
    _write_dir_spec(workdir, "e22d", manifest, "t1", t1, env=env)
    rc, out, err = run_split(bin_path, ["--workdir", wd, "todos", "run",
                                        "--file", os.path.join(workdir, "e22d"),
                                        "--json"])
    c.check("E22d todos run (dir format) rc==0", rc == 0,
            f"rc={rc} err_tail={err[-300:]}")
    wf_id = m.group(1) if (m := re.search(r"workflow_id=(\S+)", err)) else None
    c.check("E22d stderr carries workflow_id=", bool(wf_id))
    marker = os.path.join(wd, "e22d_marker.txt")
    c.check("E22d env_vars passthrough: marker carries the bound value",
            os.path.isfile(marker)
            and open(marker).read().strip() == "runtime-v1",
            f"content={open(marker).read()[:80] if os.path.isfile(marker) else None}")
    state = json_or_none(out)
    c.soft("E22d t1 passed", isinstance(state, dict)
           and ((state.get("todos") or {}).get("t1") or {}).get("status") == "passed",
           "t1 not passed")


def run_all(bin_path: str, api_key: str) -> Counter:
    """Run every E22 runtime scenario (needs a working model key)."""
    workdir = tempfile.mkdtemp(prefix="e2e-share-runtime-")
    counter = Counter()
    _e22a_success(counter, bin_path, api_key)
    _e22b_failure(counter, bin_path, api_key)
    _e22c_interrupt_resume(counter, bin_path, api_key)
    _e22d_dir_format_env(counter, bin_path, api_key, workdir)
    counter.summary("Todos runtime scenarios")
    return counter


def _main() -> int:
    bin_path = lib.resolve_bin(sys.argv[1] if len(sys.argv) > 1 else None)
    api_key = os.environ.get("ZHIPU_API_KEY", "")
    auth = os.path.expanduser("~/.local/share/opencoder/auth.json")
    if not api_key and not os.path.isfile(auth):
        print("SKIP: todos-runtime e2e needs ZHIPU_API_KEY or "
              "~/.local/share/opencoder/auth.json (see todos_contract_scenarios.py)")
        return 0
    return 1 if run_all(bin_path, api_key).failed else 0


if __name__ == "__main__":
    sys.exit(_main())
