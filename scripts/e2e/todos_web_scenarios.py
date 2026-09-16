"""Todos web-surface e2e scenarios (boots a real ``opencoder-server``) — E23.

Drives the HTTP-only todo management surface the CLI cannot reach, plus the
serve-side run surface. HARD = deterministic contract; SOFT = needs a real
model completion.

  E23a  template lifecycle over the share tree (all HARD)
        create -> {template{name,current:"v1",versions:[v1]},revision};
        duplicate create 409; list/get shapes with env_by_version {v1:null};
        validate-files {valid:true} vs 400 {error,diagnostics[{path,message,
        line,column}]}; new-version requires expected_revision or
        expected_current (409 otherwise; stale revision 409) ->
        {version:"v2",pruned:[],template,revision}; PUT version
        context.json / env.json -> 409 (versions are frozen, new-version is
        the only write path); DELETE the current version -> 409; DELETE an
        older version -> ok and todo.json stops advertising it; retention:
        once 11 versions exist the list clamps to the newest 10 and the
        response `pruned` names the dropped one whose /files then 404s.
  E23b  env lifecycle (HARD): POST /api/todo/envs -> {ok,name}; duplicate
        -> 409; GET /api/todo/envs lists it with the env_vars echo.
  E23c  run surface as deployed (compat node dispatch).  HARD: run on an
        unknown template -> 400; run on a real version without an online
        node -> 503 "no ready online node..."; unknown workflow id -> 404 on
        get/interrupt/resume/events; GET /workflows serves the list shape.
        (The full run lifecycle is node-side; a 200 run is driven
        best-effort.)

Run standalone:  python3 scripts/e2e/todos_web_scenarios.py [binary]
The serve needs a model key only for E23c's SOFT completion; the lifecycle
contracts are key-free (an unreachable endpoint yields a suspended record
and identical response shapes).
"""

from __future__ import annotations

import json
import os
import socket
import sys
import tempfile
import time
import urllib.error
import urllib.request

try:
    from . import lib
    from . import todos_contract_scenarios as tc
    from .lib import Counter
    from .web_scenarios import _E2E_TOKEN, _boot_serve, _shutdown
except ImportError:  # standalone: python3 scripts/e2e/todos_web_scenarios.py
    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    from e2e import lib
    from e2e import todos_contract_scenarios as tc
    from e2e.lib import Counter
    from e2e.web_scenarios import _E2E_TOKEN, _boot_serve, _shutdown

NAME = "web-e2e-todos"
MAX_VERSIONS = 10


def _req(method: str, url: str, body: dict | None = None, *, timeout: int = 30):
    """Bearer-authenticated request returning (status, parsed-json-or-raw)."""
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Authorization": f"Bearer {_E2E_TOKEN}"}
    if data is not None:
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode()
            return resp.status, json.loads(raw)
    except urllib.error.HTTPError as e:
        raw = e.read().decode(errors="replace")
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, {"raw": raw}


def _sse_kinds(base: str, wf_id: str, deadline_s: float = 45) -> list:
    """Read the workflow SSE stream until EOF/deadline; return (event,id)
    pairs. The stream closes once the workflow record is terminal."""
    pairs, cur = [], None
    try:
        req = urllib.request.Request(
            f"{base}/api/todo/workflows/{wf_id}/events?after=0",
            headers={"Authorization": f"Bearer {_E2E_TOKEN}"})
        with urllib.request.urlopen(req, timeout=15) as stream:
            while time.time() < deadline_s:
                try:
                    raw = stream.readline()
                except socket.timeout:
                    continue
                if not raw:
                    break
                line = raw.decode(errors="replace").strip()
                if line.startswith("event: "):
                    cur = line[len("event: "):]
                elif line.startswith("id: ") and cur:
                    pairs.append((cur, line[len("id: "):]))
                    cur = None
    except Exception:
        pass
    return pairs


def _wait_status(base: str, wf_id: str, deadline_s: float, statuses=None):
    """Poll the workflow record until it exists (and, when `statuses` is
    given, until its status is one of them). Returns the record or None."""
    end = time.time() + deadline_s
    while time.time() < end:
        try:
            st, doc = _req("GET", f"{base}/api/todo/workflows/{wf_id}")
        except Exception:
            st, doc = 0, None
        if st == 200 and isinstance(doc, dict):
            rec = doc.get("workflow") or {}
            if not statuses or rec.get("status") in statuses:
                return rec
        time.sleep(1.0)
    return None


def _e23a_template_lifecycle(c: Counter, base: str) -> None:
    print("== E23a: template lifecycle (create/validate/new-version/retention) ==")
    spec = tc.legacy_spec(NAME, [tc.todo("t1",
        instructions="Create web_e2e.txt containing ok.",
        acceptance={"criteria": "web_e2e.txt exists", "required_tool_calls": []})])
    st, doc = _req("POST", f"{base}/api/todo/templates",
                   {"name": NAME, "description": "e2e", "spec": spec})
    c.check("E23a create -> 200 {template,revision}", st == 200
            and doc.get("template", {}).get("name") == NAME
            and doc.get("template", {}).get("current") == "v1"
            and isinstance(doc.get("revision"), str),
            f"st={st} doc={json.dumps(doc)[:160]}")
    revision = doc.get("revision", "")

    st, doc = _req("POST", f"{base}/api/todo/templates",
                   {"name": NAME, "spec": spec})
    c.check("E23a duplicate create -> 409", st == 409, f"st={st} doc={doc}")

    st, doc = _req("GET", f"{base}/api/todo/templates")
    c.check("E23a list contains the created template",
            st == 200 and any(t.get("name") == NAME
                              for t in doc.get("templates", [])),
            f"st={st} templates={json.dumps(doc.get('templates', []))[:120]}")

    st, doc = _req("GET", f"{base}/api/todo/templates/{NAME}")
    c.check("E23a get serves template + revision + env_by_version {v1:null}",
            st == 200 and doc.get("template", {}).get("current") == "v1"
            and doc.get("env_by_version") == {"v1": None}
            and isinstance(doc.get("revision"), str),
            f"st={st} doc={json.dumps(doc)[:160]}")

    st, doc = _req("POST", f"{base}/api/todo/validate-files", {"spec": spec})
    c.check("E23a validate-files valid spec -> {valid:true}",
            st == 200 and doc == {"valid": True}, f"st={st} doc={doc}")
    bad = tc.legacy_spec("bad", [tc.todo("t1", instructions="x")])
    bad["todos"][0].pop("acceptance")
    st, doc = _req("POST", f"{base}/api/todo/validate-files", {"spec": bad})
    diags = doc.get("diagnostics") if isinstance(doc, dict) else None
    c.check("E23a validate-files bad spec -> 400 with diagnostics",
            st == 400 and isinstance(diags, list) and diags
            and all({"path", "message", "line", "column"} <= set(d) for d in diags),
            f"st={st} doc={json.dumps(doc)[:160]}")

    st, doc = _req("POST", f"{base}/api/todo/templates/{NAME}/new-version",
                   {"spec": spec})
    c.check("E23a new-version (edit) without expected -> 409", st == 409,
            f"st={st} doc={doc}")
    st, doc = _req("POST", f"{base}/api/todo/templates/{NAME}/new-version",
                   {"expected_revision": "rev-stale"})
    c.check("E23a new-version with stale expected_revision -> 409", st == 409,
            f"st={st} doc={doc}")
    st, doc = _req("POST", f"{base}/api/todo/templates/{NAME}/new-version",
                   {"expected_revision": revision, "note": "e2e v2"})
    c.check("E23a new-version with fresh revision -> v2 with pruned []",
            st == 200 and doc.get("version") == "v2" and doc.get("pruned") == []
            and isinstance(doc.get("revision"), str),
            f"st={st} doc={json.dumps(doc)[:160]}")
    new_revision = (doc or {}).get("revision", "")

    st, doc = _req("PUT", f"{base}/api/todo/templates/{NAME}/v2/context.json",
                   {"content": "x"})
    c.check("E23a PUT version context.json -> 409 (versions frozen)", st == 409,
            f"st={st} doc={doc}")
    st, doc = _req("PUT", f"{base}/api/todo/templates/{NAME}/v2/env.json",
                   {"env": "x"})
    c.check("E23a PUT version env.json -> 409 (binding frozen)", st == 409,
            f"st={st} doc={doc}")

    st, doc = _req("DELETE", f"{base}/api/todo/templates/{NAME}/v2")
    c.check("E23a DELETE current version -> 409", st == 409, f"st={st} doc={doc}")
    st, doc = _req("DELETE", f"{base}/api/todo/templates/{NAME}/v1")
    c.check("E23a DELETE non-current v1 -> ok", st == 200 and doc.get("ok") is True,
            f"st={st} doc={doc}")
    st, doc = _req("GET", f"{base}/api/todo/templates/{NAME}")
    c.check("E23a todo.json no longer advertises v1",
            st == 200 and [v.get("version") for v in
                           doc.get("template", {}).get("versions", [])] == ["v2"],
            f"versions={doc.get('template', {}).get('versions')}")

    pruned_first = None
    # Deleting v1 rewrote todo.json, so re-read the revision before saving on.
    st, doc = _req("GET", f"{base}/api/todo/templates/{NAME}")
    new_revision = (doc or {}).get("revision", new_revision)
    for i in range(MAX_VERSIONS):  # v3..v12: 10 more on top of v2
        st, doc = _req("POST", f"{base}/api/todo/templates/{NAME}/new-version",
                       {"expected_revision": new_revision})
        if st != 200:
            break
        new_revision = doc["revision"]
        if doc.get("pruned") and pruned_first is None:
            pruned_first = doc["pruned"][0]
    c.check("E23a retention: version list clamps to 10", st == 200
            and len(doc.get("template", {}).get("versions", [])) == MAX_VERSIONS,
            f"st={st} n={len(doc.get('template', {}).get('versions', []))}")
    c.check("E23a retention response names the pruned version",
            pruned_first is not None, f"pruned_first={pruned_first}")
    if pruned_first:
        st, doc = _req("GET",
                       f"{base}/api/todo/templates/{NAME}/{pruned_first}/files")
        c.check("E23a pruned version's files are gone (404)", st == 404,
                f"st={st} doc={doc}")


def _e23b_env_lifecycle(c: Counter, base: str) -> None:
    print("== E23b: env lifecycle (create / duplicate / list) ==")
    env = {"name": "web-e2e-env", "description": "e2e",
           "env_vars": {"E2E_WEB_MARKER": "web-v1"}}
    st, doc = _req("POST", f"{base}/api/todo/envs", env)
    c.check("E23b create env -> {ok,name}", st == 200
            and doc.get("ok") is True and doc.get("name") == "web-e2e-env",
            f"st={st} doc={doc}")
    st, doc = _req("POST", f"{base}/api/todo/envs", env)
    c.check("E23b duplicate env -> 409", st == 409, f"st={st} doc={doc}")
    st, doc = _req("GET", f"{base}/api/todo/envs")
    listed = [e for e in doc.get("envs", []) if e.get("name") == "web-e2e-env"]
    c.check("E23b list contains env with env_vars echo",
            st == 200 and listed
            and listed[0].get("env_vars", {}).get("E2E_WEB_MARKER") == "web-v1",
            f"st={st} listed={json.dumps(listed)[:120]}")


def _e23c_run_surface(c: Counter, base: str) -> None:
    """Run surface as deployed on `opencoder-server`: template run dispatches
    to an online node (compat router). On this node-less server the
    deterministic contracts are the template-resolve errors, the node
    selection refusal, and the unknown-id 404s; the full runtime lifecycle
    runs node-side (covered by the CLI E22 suite and control's own e2e), so
    a 200 run (a node really is online) is driven best-effort."""
    print("== E23c: run surface (node-less serve contract) ==")
    st, doc = _req("GET", f"{base}/api/todo/templates/{NAME}")
    version = (doc or {}).get("template", {}).get("current", "v1")
    c.check("E23c current version readable for run", st == 200
            and isinstance(version, str) and version.startswith("v"),
            f"st={st} version={version}")

    st, doc = _req("POST",
                   f"{base}/api/todo/templates/no-such-template/{version}/run",
                   {})
    c.check("E23c run on unknown template -> 400 naming it",
            st == 400 and "no-such-template" in str(doc.get("error", "")),
            f"st={st} doc={doc}")

    st, doc = _req("POST",
                   f"{base}/api/todo/templates/{NAME}/{version}/run", {})
    if st == 200 and isinstance(doc.get("workflow_id"), str):
        _run_surface_online(c, base, doc["workflow_id"])  # node online: best effort
    else:
        c.check("E23c run without online node -> 503 (node dispatch)",
                st == 503 and "no ready online node" in str(doc.get("error", "")),
                f"st={st} doc={doc}")

    st, doc = _req("GET", f"{base}/api/todo/workflows/does-not-exist")
    c.check("E23c unknown workflow -> 404", st == 404, f"st={st} doc={doc}")
    for verb in ("interrupt", "resume"):
        st, doc = _req("POST", f"{base}/api/todo/workflows/does-not-exist/{verb}",
                       {})
        c.check(f"E23c unknown workflow {verb} -> 404", st == 404,
                f"st={st} doc={doc}")
    st, doc = _req("GET",
                   f"{base}/api/todo/workflows/does-not-exist/events?after=0")
    c.check("E23c unknown workflow events -> 404", st == 404, f"st={st} doc={doc}")
    st, doc = _req("GET", f"{base}/api/todo/workflows")
    c.check("E23c workflows list shape", st == 200 and "workflows" in doc,
            f"st={st} doc={doc}")


def _run_surface_online(c: Counter, base: str, wf_id: str) -> None:
    """Best-effort full surface when a node IS online (non-standard here)."""
    rec = _wait_status(base, wf_id, 60)
    c.soft("E23c run -> workflow record visible", isinstance(rec, dict),
           f"rec={json.dumps(rec)[:120] if rec else None}")
    pairs = _sse_kinds(base, wf_id)
    c.check("E23c SSE /events carries event:+id: frames", len(pairs) >= 1,
            f"pairs={pairs[:6]}")
    st, doc = _req("GET", f"{base}/api/todo/workflows/does-not-exist")
    c.check("E23c unknown workflow still 404s", st == 404, f"st={st} doc={doc}")


def run_all(bin_path: str, api_key: str) -> Counter:
    """Run every E23 todos-web scenario against one isolated serve."""
    os.environ["ZHIPU_API_KEY"] = api_key  # serve subprocesses inherit env
    counter = Counter()
    cfg = lib.make_config(api_key=api_key)
    # Isolate the share tree (templates/envs) instead of the global
    # ~/.opencoder/share, so the suite is idempotent across runs.
    cfg["agent"] = {"share_dir": tempfile.mkdtemp(prefix="e2e-share-web-")}
    booted = _boot_serve(bin_path, cfg, "todos web scenarios")
    if booted is None:
        counter.check("serve started and /api/health is up", False)
        counter.summary("Todos web scenarios")
        return counter
    proc, base, _port, _webdir = booted
    counter.check("serve started and /api/health is up", True)
    try:
        _e23a_template_lifecycle(counter, base)
        _e23b_env_lifecycle(counter, base)
        _e23c_run_surface(counter, base)
    finally:
        _shutdown(proc)
    counter.summary("Todos web scenarios")
    return counter


def _main() -> int:
    bin_path = lib.resolve_bin(sys.argv[1] if len(sys.argv) > 1 else None)
    return 1 if run_all(bin_path, os.environ.get("ZHIPU_API_KEY", "")).failed else 0


if __name__ == "__main__":
    sys.exit(_main())
