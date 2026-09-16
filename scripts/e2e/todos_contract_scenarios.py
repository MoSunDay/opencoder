"""Key-free todos-contract e2e scenarios (E21).

These scenarios NEVER call the LLM: they exercise the `todos validate`
contract and the observation-command error contract, so the module runs in
any shell without credentials — included in EVERY mode of scripts/e2e_glm.py
(same gate-free slot as config_scenarios). All asserts are HARD: validate and
the observation commands are deterministic store/filesystem surfaces.

  E21a   validate accepts the three input shapes: legacy single-file JSON,
         the full directory format, and the legacy `context.json` directory
         (converted through directory::encode). Validate must also stay
         store-free: it runs BEFORE Store init, so no opencoder.db may appear
         under the (overridden) data root.
  E21b   validate rejects malformed directory specs with path-attributed
         diagnostics (`path:line:column: message` on stderr, rc 1): missing
         file, stray file, illegal/duplicate todo ids, cycle, max_attempts=0,
         non-object arguments_contains, non-primary agent.
  E21c   env binding (load_bound): a directory spec bound via env.json to an
         `agent.share_dir` env context validates; a missing env, a missing
         tool reference, and a malformed env_vars key each fail with the
         Chinese diagnostic the share layer raises.
  E21d   observation commands on an empty/failed store: `list` (pretty+json),
         show/events/resume/interrupt on an unknown id all exit 1 naming the
         id; `run` vs an unreachable endpoint fails fast AND still records
         the workflow as `suspended` (runtime-error suspension).

Run standalone:  python3 scripts/e2e/todos_contract_scenarios.py [binary]
"""

from __future__ import annotations

import glob
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

try:
    from . import lib
    from .lib import Counter, json_or_none, run_split
except ImportError:  # standalone: python3 scripts/e2e/todos_contract_scenarios.py
    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    from e2e import lib
    from e2e.lib import Counter, json_or_none, run_split

# Real-looking but never-valid credential: nothing here contacts a network.
FAKE_KEY = "sk-e2e-1234567890abcdef"
UNREACHABLE = "http://127.0.0.1:9/v1"


def _cfg() -> dict:
    """Minimal config: literal fake key (no env interpolation), no compaction
    tuning — validate never resolves an endpoint, and E21d's run must fail on
    the unreachable base_url, never on a missing credential."""
    return {
        "model": "zhipuai-coding-plan/glm-5.2",
        "provider": {
            "base_url": "https://open.bigmodel.cn/api/coding/paas/v4",
            "api_key": FAKE_KEY,
        },
        "reasoning_effort": "medium",
        "max_tokens": 16384,
    }


def todo(todo_id: str, **over) -> dict:
    todo = {
        "id": todo_id,
        "title": f"todo {todo_id}",
        "requirement_background": "e2e contract todo",
        "instructions": "Do the thing.",
        "depends_on": [],
        "agent": "act",
        "max_attempts": 3,
        "acceptance": {"criteria": "the thing is done", "required_tool_calls": []},
    }
    todo.update(over)
    return todo


def legacy_spec(workflow_id: str, todos: list[dict], **over) -> dict:
    spec = {
        "schema_version": 1,
        "id": workflow_id,
        "name": f"todos {workflow_id}",
        "objective": "Contract e2e objective.",
        "constraints": [],
        "metadata": {},
        "todos": todos,
    }
    spec.update(over)
    return spec


def _write_dir_spec(workdir: str, name: str, spec: dict, *, env=None, extra=None) -> str:
    """Write the full directory format for `spec` under <workdir>/<name>."""
    root = os.path.join(workdir, name)
    files: dict[str, str] = {
        "workflow.json": json.dumps({
            "schema_version": spec["schema_version"],
            "id": spec["id"],
            "name": spec["name"],
            "todos": [t["id"] for t in spec["todos"]],
            "constraints": spec.get("constraints", []),
            "metadata": spec.get("metadata", {}),
        }),
        "objective.md": spec["objective"],
        "env.json": json.dumps({"env": env}),
    }
    for t in spec["todos"]:
        base = f"todos/{t['id']}"
        files[f"{base}/task.json"] = json.dumps({
            "title": t["title"],
            "agent": t["agent"],
            "max_attempts": t["max_attempts"],
            "depends_on": t.get("depends_on", []),
            "required_tool_calls": t["acceptance"].get("required_tool_calls", []),
            "metadata": t.get("metadata", {}),
        })
        files[f"{base}/context.md"] = t["requirement_background"]
        files[f"{base}/instructions.md"] = t["instructions"]
        files[f"{base}/acceptance.md"] = t["acceptance"]["criteria"]
    for rel, content in (extra or {}).items():
        files[rel] = content
    for rel, content in files.items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(content)
    return root


def _run(bin_path: str, wd: str, args: list[str], *, env_extra=None, timeout=120):
    env = dict(os.environ)
    if env_extra:
        env.update(env_extra)
    try:
        p = subprocess.run([bin_path, "--workdir", wd] + args, capture_output=True,
                           text=True, timeout=timeout, env=env)
        return p.returncode, p.stdout or "", p.stderr or ""
    except subprocess.TimeoutExpired:
        return 124, "", f"TIMEOUT after {timeout}s"


def _no_db(data_home: str) -> bool:
    return not glob.glob(os.path.join(data_home, "opencoder", "*", "opencoder.db"))


def _e21a_valid_inputs(c: Counter, bin_path: str) -> None:
    print("== E21a: validate accepts legacy file / directory / context.json dir ==")
    cfg = _cfg()
    wd = lib.seed_workdir(cfg)
    spec = legacy_spec("contract-ok", [todo("t1")])

    # 1) legacy single-file JSON.
    path = os.path.join(wd, "single.json")
    lib.write_file(wd, "single.json", json.dumps(spec))
    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file", path])
    doc = json_or_none(out)
    c.check("E21a legacy single file: rc 0 + {valid:true,...}",
            rc == 0 and isinstance(doc, dict) and doc.get("valid") is True
            and doc.get("workflow_id") == "contract-ok" and doc.get("todo_count") == 1,
            f"rc={rc} out={out[:120]} err={err[-200:]}")

    # 2) full directory format.
    dpath = _write_dir_spec(wd, "dirspec", spec)
    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file", dpath])
    doc = json_or_none(out)
    c.check("E21a directory format: rc 0 + valid:true",
            rc == 0 and isinstance(doc, dict) and doc.get("valid") is True,
            f"rc={rc} out={out[:120]} err={err[-200:]}")

    # 3) legacy context.json directory (converted via directory::encode).
    legacy_dir = os.path.join(wd, "legacydir")
    os.makedirs(legacy_dir)
    with open(os.path.join(legacy_dir, "context.json"), "w") as f:
        f.write(json.dumps(spec))
    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file", legacy_dir])
    doc = json_or_none(out)
    c.check("E21a legacy context.json dir: rc 0 + valid:true",
            rc == 0 and isinstance(doc, dict) and doc.get("valid") is True,
            f"rc={rc} out={out[:120]} err={err[-200:]}")

    # Validate never initializes the Store: no opencoder.db under the data root.
    data_home = tempfile.mkdtemp(prefix="opencoder_e2e_data_")
    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file", path],
                        env_extra={"XDG_DATA_HOME": data_home})
    c.check("E21a validate leaves the data root store-free (no opencoder.db)",
            rc == 0 and _no_db(data_home),
            f"rc={rc} dbs={glob.glob(os.path.join(data_home, 'opencoder', '*', 'opencoder.db'))}")
    shutil.rmtree(data_home, ignore_errors=True)


def _e21b_invalid_diagnostics(c: Counter, bin_path: str) -> None:
    print("== E21b: validate rejects malformed specs with path diagnostics ==")
    wd = lib.seed_workdir(_cfg())
    ok = todo("t1")
    # (label, spec builder, extra files, workflow.json todos override, needles)
    cases = [
        ("missing objective.md", lambda: legacy_spec("m1", [ok]), None, None,
         ["objective.md", "必需的 Markdown 内容不能为空"]),
        ("stray extra file", lambda: legacy_spec("m2", [ok]),
         {"stray.txt": "junk"}, None, ["stray.txt", "文件不属于 TODO 框架定义"]),
        ("illegal todo id t/1", lambda: legacy_spec("m2x", [ok]), None, ["t/1"],
         ["workflow.json", "非法 TODO 目录名: t/1"]),
        ("dotdot todo id", lambda: legacy_spec("m2y", [ok]), None, None,
         ["workflow.json", "非法 TODO 目录名: .."]),
        ("duplicate todo id", lambda: legacy_spec("m3", [ok]), None,
         ["t1", "t1"], ["workflow.json", "重复 TODO: t1"]),
        ("dependency cycle", lambda: legacy_spec("m4", [
            todo("t1", depends_on=["t2"]), todo("t2", depends_on=["t1"])]),
         None, None, ["cycle"]),
        ("max_attempts=0",
         lambda: legacy_spec("m5", [todo("t1", max_attempts=0)]), None, None,
         ["todos/t1/task.json", "max_attempts must be positive"]),
        ("arguments_contains non-object",
         lambda: legacy_spec("m6", [todo("t1", acceptance={
             "criteria": "x", "required_tool_calls": [
                 {"name": "bash", "arguments_contains": []}]})]),
         None, None, ["todos/t1/task.json", "invalid required tool call"]),
        ("non-primary agent",
         lambda: legacy_spec("m7", [todo("t1", agent="workflow")]), None, None,
         ["todos/t1/task.json", "workflow agent"]),
    ]
    for index, (label, build, extra, ids_override, needles) in enumerate(cases):
        spec = build()
        if label == "dotdot todo id":
            spec["todos"][0]["id"] = ".."
        path = _write_dir_spec(wd, f"bad-{index}", spec, extra=extra)
        if label in ("illegal todo id t/1", "duplicate todo id"):
            wj = os.path.join(path, "workflow.json")
            doc = json.load(open(wj))
            doc["todos"] = ids_override
            json.dump(doc, open(wj, "w"))
        if label == "missing objective.md":
            os.remove(os.path.join(path, "objective.md"))
        rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file", path])
        c.check(f"E21b {label}: exit 1", rc == 1, f"rc={rc} err={err[-160:]}")
        for needle in needles:
            c.check(f"E21b {label}: diagnostic names {needle!r}", needle in err,
                    f"err={err[-240:]}")


def _seed_share(env_name: str) -> str:
    share = tempfile.mkdtemp(prefix="opencoder_e2e_share_")
    ctx_dir = os.path.join(share, "env", env_name)
    os.makedirs(ctx_dir)
    with open(os.path.join(ctx_dir, "context.json"), "w") as f:
        json.dump({"name": env_name, "description": "e2e",
                   "tools": [], "env_vars": {"E2E_MARKER": "contract-v1"}}, f)
    return share


def _e21c_env_binding(c: Counter, bin_path: str) -> None:
    print("== E21c: env binding via agent.share_dir (load_bound) ==")
    share = _seed_share("e2eenv")
    cfg = _cfg()
    cfg["agent"] = {"share_dir": share}
    wd = lib.seed_workdir(cfg)
    spec = legacy_spec("bound", [todo("t1")])

    def env_json(binding: dict) -> str:
        d = _write_dir_spec(wd, f"env-{binding.get('env') or 'null'}", spec,
                            env=binding.get("env"))
        with open(os.path.join(d, "env.json"), "w") as f:
            json.dump(binding, f)
        return d

    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file",
                                       env_json({"env": "e2eenv"})])
    doc = json_or_none(out)
    c.check("E21c legal binding: valid:true",
            rc == 0 and isinstance(doc, dict) and doc.get("valid") is True,
            f"rc={rc} err={err[-200:]}")

    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file",
                                       env_json({"env": "no-such-env"})])
    c.check("E21c missing env: exit 1 naming the env",
            rc == 1 and "环境不存在" in err and "no-such-env" in err,
            f"rc={rc} err={err[-240:]}")

    ghost = os.path.join(share, "env", "ghost-tool")
    os.makedirs(ghost)
    with open(os.path.join(ghost, "context.json"), "w") as f:
        json.dump({"name": "ghost-tool", "tools": ["/agent/tools/v3/nope"],
                   "env_vars": {}}, f)
    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file",
                                       env_json({"env": "ghost-tool"})])
    c.check("E21c missing tool reference: exit 1 with 工具不存在",
            rc == 1 and "工具不存在" in err,
            f"rc={rc} err={err[-240:]}")

    badvars = os.path.join(share, "env", "badvars")
    os.makedirs(badvars)
    with open(os.path.join(badvars, "context.json"), "w") as f:
        json.dump({"name": "badvars", "tools": [], "env_vars": {"lower": "x"}}, f)
    rc, out, err = _run(bin_path, wd, ["todos", "validate", "--file",
                                       env_json({"env": "badvars"})])
    c.check("E21c malformed env_vars key: exit 1 with env_vars diagnostic",
            rc == 1 and "env_vars" in err,
            f"rc={rc} err={err[-240:]}")
    shutil.rmtree(share, ignore_errors=True)


def _e21d_observation_contract(c: Counter, bin_path: str) -> None:
    print("== E21d: observation commands on empty store + unreachable run ==")
    cfg = _cfg()
    cfg["provider"]["base_url"] = UNREACHABLE
    wd = lib.seed_workdir(cfg)
    ghost = "ghosts-workflow-id"

    rc, out, _ = run_split(bin_path, ["--workdir", wd, "todos", "list"], 60)
    c.check("E21d todos list on empty store: exit 0, no output",
            rc == 0 and out.strip() == "", f"rc={rc} out={out[:120]}")
    rc, out, _ = run_split(bin_path, ["--workdir", wd, "todos", "list", "--json"], 60)
    c.check("E21d todos list --json on empty store: []",
            rc == 0 and json_or_none(out) == [], f"rc={rc} out={out[:120]}")

    for label, args in (
        ("show", ["todos", "show", ghost]),
        ("show --json", ["todos", "show", ghost, "--json"]),
        ("events", ["todos", "events", ghost]),
        ("resume --json", ["todos", "resume", ghost, "--json"]),
        ("interrupt", ["todos", "interrupt", ghost]),
    ):
        rc, out, err = run_split(bin_path, ["--workdir", wd] + args, 60)
        c.check(f"E21d unknown id: {label} exits 1 naming the id",
                rc == 1 and ghost in err,
                f"rc={rc} err={err[-200:]}")

    # run against an unreachable endpoint: fast clean failure, no hang, and
    # the workflow is still persisted as suspended (runtime-error suspension).
    spec = legacy_spec("unreachable", [todo("t1")])
    path = os.path.join(wd, "unreachable.json")
    lib.write_file(wd, "unreachable.json", json.dumps(spec))
    started = time.time()
    rc, out, err = run_split(bin_path, ["--workdir", wd, "todos", "run",
                                        "--file", path, "--json"], 120)
    elapsed = time.time() - started
    c.check("E21d todos run vs unreachable endpoint: nonzero exit, fast",
            rc != 0 and elapsed < 110, f"rc={rc} elapsed={elapsed:.1f}s")
    rc_l, out_l, _ = run_split(bin_path, ["--workdir", wd, "todos", "list",
                                          "--json"], 60)
    workflows = json_or_none(out_l)
    suspended = [w for w in (workflows or []) if w.get("status") == "suspended"]
    c.check("E21d failed run persisted the workflow as suspended",
            rc_l == 0 and len(suspended) == 1,
            f"rc={rc_l} workflows={workflows!r}"[:240])
    if suspended:
        wf_id = suspended[0]["id"]
        rc_s, out_s, err_s = run_split(bin_path, ["--workdir", wd, "todos",
                                                  "show", wf_id, "--json"], 60)
        doc = json_or_none(out_s)
        state = (doc or {}).get("state") or {}
        c.check("E21d suspended state carries terminal_reason",
                rc_s == 0 and isinstance(doc, dict)
                and (doc.get("state") or {}).get("status") == "suspended"
                and bool(state.get("terminal_reason")),
                f"rc={rc_s} err={err_s[-160:]} out={out_s[:120]}")


def run_all(bin_path: str) -> Counter:
    """Run every key-free todos-contract scenario. NOTE: no api_key parameter
    — nothing here may call the LLM."""
    c = Counter()
    _e21a_valid_inputs(c, bin_path)
    _e21b_invalid_diagnostics(c, bin_path)
    _e21c_env_binding(c, bin_path)
    _e21d_observation_contract(c, bin_path)
    c.summary("Todos contract scenarios")
    return c


def _main() -> int:
    import argparse

    ap = argparse.ArgumentParser(
        description="opencoder key-free todos-contract e2e (E21; never calls the LLM)"
    )
    ap.add_argument("binary", nargs="?", default=None, help="path to the opencoder binary")
    args = ap.parse_args()

    bin_path = lib.resolve_bin(args.binary)
    if not os.path.isfile(bin_path):
        print(f"FAIL: binary not found: {bin_path}", file=sys.stderr)
        return 2
    total = run_all(bin_path)
    print("\n" + "=" * 60)
    print(f"todos contract e2e result: {total.passed} passed, {total.failed} failed, "
          f"{total.skipped} skipped")
    print("=" * 60)
    return 1 if total.failed else 0


if __name__ == "__main__":
    sys.exit(_main())
