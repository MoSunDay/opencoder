# Review-ops scripts - what each script must guarantee

Example node-side ops for the review release-gate DAGs. The wasm step
modules (crates/dag-review-tools) parse whatever these scripts print, so
the contracts below are load-bearing, not style advice.

## Shared rules (all scripts)

- Launch is argv-only, NO shell, cwd = the run's context root, and the
  child environment contains ONLY the `dag.ops` `env_keys` vars
  (`env_clear()`): set `PATH` at the top of the script and use absolute
  paths for everything else.
- Secrets come ONLY from the whitelisted env vars (`REVIEW_DEPLOY_TOKEN`,
  `REVIEW_GITHUB_TOKEN`). Never echo values; never bake them into files.
- stdout+stderr are captured TOGETHER and capped at 256 KiB per op run;
  overflow kills the whole process group and errors the step. Print
  one-line progress, never full build/service logs.
- Exit codes: `0` = op succeeded. Non-zero reaches the wasm module, which
  fails the step (fail-closed). Host-side kills (timeout/overflow/cancel)
  error the step outright, and partial output still lands in the evidence
  log `<step_dir>/ops/<op_id>.log`.

## env_eob_deploy.sh / env_baremetal_deploy.sh

- Take the workspace path as `$1` (default `/data00/github/opencoder`);
  endpoint host/port via `ENDPOINT_HOST`/`ENDPOINT_PORT` (defaults
  `127.0.0.1:18080` eob, `127.0.0.1:18090` baremetal).
- MUST end with a single JSON tail line - the LAST non-empty stdout line:

  ```json
  {"endpoint": "http://127.0.0.1:18080"}
  ```

  `{"host": "127.0.0.1", "port": 18080}` also parses. The module then
  probes `<endpoint>/ready` (eob) or `<endpoint>/healthz` (baremetal)
  until 2xx; anything else (including a missing tail line) is a `down`
  verdict and a failed step.

## harness_run.sh

- One script, two ops: `harness_run.sh quick` and `harness_run.sh full`
  (mode from `$1`; anything else exits 2). `quick` skips the slow `e2e`
  stage.
- Streams NDJSON stage lines, one JSON object per line; non-object lines
  around them are ignored:

  ```json
  {"stage": "build", "status": "ok"}
  {"stage": "unit", "status": "fail", "detail": "cargo test exited 1: tests/session.rs:88"}
  ```

- Verdict rule (enforced by the module, mirrored by the script): exit 0
  AND at least one stage AND every stage `status` exactly `"ok"`. A
  missing or unrecognized `status` counts as `fail`.
- `./harness_run.sh selftest-fail` prints one failing stage line with a
  `detail` and exits 1 - use it to eyeball the fail shape and to verify
  node wiring without a real regression.
