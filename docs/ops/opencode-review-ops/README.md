Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# opencode-review-ops - node-side ops for the review release-gate DAGs

Operational templates for the code-review release gates: example deploy and
harness scripts that plug into a node's `dag.ops` whitelist, plus a node
config fragment and the publish/dispatch cookbook for the deterministic wasm
modules (`env_eob_up` / `env_baremetal_up` / `harness_runner`, crate
`opencoder-dag-review-tools`).

## How it works / trust model

The review DAGs are deterministic wasm modules. WASI preview 1 has no
network and no subprocesses, so every heavy action (a deploy, a harness run)
is delegated to the NODE through the `opencoder_run_op` host import. The
capability surface is deliberately tiny and fail-closed
(`crates/dag-runtime/src/exec/wasm/host_imports/ops.rs`):

- Op ids must match `[A-Za-z0-9_-]{1,64}` and MUST be registered per node in
  config `dag.ops`. An unregistered op id errors the wasm step (fail-closed)
  instead of running something.
- The registered `command` string is whitespace-split into argv. There is NO
  shell: no pipes, no `$VAR` expansion, no globbing. Use absolute paths - the
  child is spawned with `env_clear()`, so it has no `PATH` unless you declare
  `PATH` in `env_keys`.
- The child runs with cwd = the run's context root, stdin `/dev/null`, and
  ONLY the environment variables named in `env_keys` copied from the node
  environment. Tokens never appear in DAG specs, wasm modules, or the wire
  protocol - they live in the node env (systemd `Environment=`, unit drop-in)
  and reach the child verbatim.
- Captured stdout+stderr is bounded to 256 KiB (`OP_LOG_LIMIT_BYTES`).
  Overflow kills the whole child process group (SIGKILL on the pgid) and
  errors the step. Keep op output small: summaries, not logs.
- Every op run writes evidence at `<step_dir>/ops/<op_id>.log`, even when the
  op is killed. The module reads it back to parse results:

  ```
  # op env_eob_deploy exit=0 duration_ms=4213 timeout_secs=900 truncated=false
  -- output --
  ...raw captured stdout+stderr...
  ```

  A killed op records `# op <id> killed (Timeout|Overflow|Cancel)` instead of
  `exit=N`. Default wall-clock budget is 600 s; the registration's
  `timeout_secs` overrides it. Timeout and cancel kills take down the whole
  process group, so shell wrappers that fork children cannot stall the pipes.
- A non-zero op exit code is NOT automatically a step error: the host returns
  it to the module, which applies the contract below. Unknown op, spawn
  failure, timeout and output overflow ARE step errors.

## Install (node side)

1. Copy the scripts to a fixed location and make them executable:

   ```sh
   install -d /opt/opencode-review-ops
   install -m 0755 docs/ops/opencode-review-ops/scripts/*.sh /opt/opencode-review-ops/
   ```

2. Merge the `dag` block of `node-config.example.json` into the node's
   config file (the `dag` block is node-local; the DAG spec itself never
   carries sandbox or op details). Adjust the script paths and ports.

3. Provide the secrets in the NODE environment only (systemd unit
   `Environment=REVIEW_DEPLOY_TOKEN=...`, `EnvironmentFile=`, or equivalent).
   JSON has no comments and must hold no secrets: `env_keys` names the vars,
   the values stay out of every file committed to git.

4. Sanity-check one op by hand before wiring a gate (same argv the runtime
   would use, no shell):

   ```sh
   /opt/opencode-review-ops/harness_run.sh selftest-fail   # prints one FAIL stage, exit 1
   ```

## Build and publish the wasm modules

The three step modules build once and publish into the server's versioned
wasm pool (`POST /api/dag/wasm`, base64 body; duplicate names 409, new
versions via `PUT /api/dag/wasm/<name>`, rollback via
`POST /api/dag/wasm/<name>/rollback`). Pool names allow `_` (charset
`[A-Za-z0-9_.-]`, max 48 chars; module bytes capped at 32 MiB).

```sh
cargo build -p opencoder-dag-review-tools --target wasm32-wasip1
# binaries: target/wasm32-wasip1/debug/{env_eob_up,env_baremetal_up,harness_runner}.wasm

base64 -w0 target/wasm32-wasip1/debug/env_eob_up.wasm > /tmp/env_eob_up.b64
curl -sS -X POST "$SERVER/api/dag/wasm" \
  -H "authorization: Bearer $TOKEN" -H "content-type: application/json" \
  -d "$(jq -n --rawfile b64 /tmp/env_eob_up.b64 \
        '{name:"env_eob_up", description:"review gate: eob env up", wasm_b64:$b64}')"
# -> 201 {"ok":true,"name":"env_eob_up","version":1}
```

Repeat for `env_baremetal_up` and `harness_runner`. A later rebuild goes out
as a new version (never reuses a version number):

```sh
curl -sS -X PUT "$SERVER/api/dag/wasm/harness_runner" \
  -H "authorization: Bearer $TOKEN" -H "content-type: application/json" \
  -d "$(jq -n --rawfile b64 /tmp/harness_runner.b64 \
        '{description:"review gate: harness runner v2", wasm_b64:$b64}')"
```

## Auto-seeded DAGs

The server seeds these defs at startup (skip-if-name-exists; operator edits
made through `POST /api/dag/defs` are never overwritten by a restart):

| DAG def | Steps (wasm command, step timeout) |
| --- | --- |
| `review-full-acceptance` | `env-eob-up` (`env_eob_up.wasm`, 900s) -> `env-baremetal-up` (`env_baremetal_up.wasm`, 900s) -> `harness-full` (`harness_runner.wasm full`, 3600s) |
| `review-harness-quick` | `harness-quick` (`harness_runner.wasm quick`, 1800s) |
| `review-code-quick` | 4 agent steps: `review-triage` -> `review-risks` -> `review-verdict` -> `review-report` (1800s each) |

Dispatch (blank or omitted `node_id` = any node):

```sh
curl -sS -X POST "$SERVER/api/dag/defs/review-harness-quick/dispatch" \
  -H "authorization: Bearer $TOKEN" -H "content-type: application/json" \
  -d '{"node_id":""}'
# -> {"run_id":"..."}
```

Keep the spec's step `timeout_secs` >= the op's registered `timeout_secs`:
the step dies first otherwise and the op kill races the step cancel.

## Op and script contract reference

| Op id | Called by module | Output contract | Success gate |
| --- | --- | --- | --- |
| `env_eob_deploy` | `env_eob_up` | LAST stdout line is JSON `{"endpoint":"http://host:port"}` (or `{"host":...,"port":...}`) | op exit 0 AND endpoint answers `/ready` with 2xx |
| `env_baremetal_deploy` | `env_baremetal_up` | same tail-JSON contract | op exit 0 AND endpoint answers `/healthz` with 2xx |
| `harness_run_quick` | `harness_runner` arg `quick` | NDJSON stage lines (below) | op exit 0 AND >= 1 stage AND every stage `ok` |
| `harness_run_full` | `harness_runner` arg `full` | same NDJSON contract | same |

Stage lines are one JSON object per line; any unparsable line around them is
ignored (human-readable progress is fine):

```
{"stage": "build", "status": "ok"}
{"stage": "unit", "status": "fail", "detail": "cargo test exited 1: tests/session.rs:88"}
```

A missing or unrecognized `status` counts as `fail` (fail-closed). Details in
`scripts/README.md`.

## Adding a new op (checklist)

1. Write the script: argv-only friendly, absolute paths (or set `PATH` at the
   top), read tokens only from env vars, stdout+stderr under 256 KiB total.
2. Register it in the node config `dag.ops` with an op id matching
   `[A-Za-z0-9_-]{1,64}`, the absolute command, `env_keys`, and a realistic
   `timeout_secs`. Restart/reload the node so the registry reloads.
3. Decide the result contract: tail-JSON line (env-style) or NDJSON stages
   (harness-style). Document it next to the script.
4. Extend or fork a wasm module in `crates/dag-review-tools` to call the op
   id and parse the evidence; add a unit test for the parser.
5. Rebuild, publish a new wasm pool version, dispatch a scratch DAG against
   a scratch node before wiring the op into a production gate.
