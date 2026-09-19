# kb-index

Deterministic knowledge-base index step for the `code-review` release-gate
DAG (M2). Pure `std`, zero dependencies, zero LLM calls: it walks the
read-only knowledge mount (`OPENCODER_KNOWLEDGE_DIR`, default
`/workspace/knowledge`), records a sorted file manifest, probes that the
mount is really read-only, and writes the step artifacts into
`OPENCODER_STEP_DIR` (default `/workspace/context/kb-index`):

- `manifest.json` — `[{ "path": ..., "size": ... }, ...]`, sorted by path,
  capped at 20 000 entries / 1 MiB serialized (`truncated: true` on trip).
- `output.json` — `{ entries, truncated, manifest, knowledge_root,
  read_only_probe }`; the `read_only_probe.write_failed: true` result is the
  expected evidence that the knowledge mount is read-only.
- one summary JSON line on stdout.

## Build

```sh
cargo build --target wasm32-wasip1 --release
```

The crate is excluded from the workspace (see the root `Cargo.toml`
`exclude` list) and has no dependencies, so a host-target
`cargo check --manifest-path examples/dag-modules/kb-index/Cargo.toml`
must always pass. Any IO error other than the expected read-only probe
failure exits 1 with the message on stderr.
