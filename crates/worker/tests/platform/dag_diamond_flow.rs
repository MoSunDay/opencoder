//! Diamond workflow end-to-end across the full platform stack: a wasm `a`
//! fans out into wasm `b`/`c` that each really READ `a`'s context.json and
//! transform its output, converging in an agent `d` that summarizes all
//! upstream outputs. Real control app + WS node + node wasm runtime + real
//! agent session; only the LLM is scripted (zero real models, no network).

use super::*;

/// The DAG agent step captures its transcript from TextDelta frames, so a
/// scripted answer needs the delta plus the terminal Completed event.
fn completed(text: String) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.clone()),
        LlmEvent::Completed {
            text,
            tool_calls: vec![],
            usage: None,
        },
    ]
}

/// Compile a WAT source and stage it into the node's shared module library
/// `<data>/dag/_modules/<file>` (staging before `Create` accepts the run).
fn stage_module(data_dir: &std::path::Path, file: &str, wat: &str) {
    let library = data_dir.join("dag").join("_modules");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(library.join(file), wat::parse_str(wat).unwrap()).unwrap();
}

/// Step `a`: write `{"value":1}` to `a/output.json` through the preopen at
/// fd 3 and echo it on stdout — the path_open/fd_write shape of
/// `exec/wasm/tests.rs::OUTPUT_JSON_WAT`.
fn echo_wat() -> String {
    r#"(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_close"
    (func $fd_close (param i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 32) "a/output.json")
  (data (i32.const 64) "{\"value\":1}")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 0))
    (drop (call $path_open
      (i32.const 3) (i32.const 0) (i32.const 32) (i32.const 13)
      (i32.const 9) (i64.const 64) (i64.const 0) (i32.const 0) (i32.const 0)))
    (i32.store (i32.const 16) (i32.const 64))
    (i32.store (i32.const 20) (i32.const 11))
    (drop (call $fd_write (i32.load (i32.const 0)) (i32.const 16) (i32.const 1) (i32.const 24)))
    (drop (call $fd_close (i32.load (i32.const 0))))
    (drop (call $fd_write (i32.const 1) (i32.const 16) (i32.const 1) (i32.const 24)))))"#
        .to_string()
}

/// Steps `b`/`c`: read `<step>/context.json`, scan BACKWARDS for the last
/// ASCII digit (the upstream `value` is the final number in the pretty
/// context), splice `digit + delta` over the `0` placeholder of a
/// `{"value":0}` template at offset 64+9, then write `<step>/output.json`
/// and echo on stdout.
///
/// Memory layout: 0 `<step>/context.json` · 32 `<step>/output.json` ·
/// 64 template (11 bytes) · 128 read buffer (1 KiB) · 1152 nread ·
/// 1160 fd slot · 1168/1172 read iovec · 1184/1188 write iovec ·
/// 1192 fd_write errno scratch.
fn delta_wat(step: &str, delta: u32) -> String {
    let ctx_path = format!("{step}/context.json");
    let out_path = format!("{step}/output.json");
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_read"
    (func $fd_read (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_close"
    (func $fd_close (param i32) (result i32)))
  (memory (export "memory") 2)
  (data (i32.const 0) "{ctx_path}")
  (data (i32.const 32) "{out_path}")
  (data (i32.const 64) "{{\"value\":0}}")
  (func (export "_start")
    (local $i i32)
    (local $digit i32)
    ;; open the upstream context read-only: oflags=0 (existing file),
    ;; rights=FD_READ (1<<1) — wasmtime maps that bit to the READ flag.
    (i32.store (i32.const 1160) (i32.const 0))
    (drop (call $path_open
      (i32.const 3) (i32.const 0) (i32.const 0) (i32.const {ctx_len})
      (i32.const 0) (i64.const 2) (i64.const 0) (i32.const 0) (i32.const 1160)))
    ;; fd_read the whole context into the buffer at 128 (nread at 1152).
    (i32.store (i32.const 1168) (i32.const 128))
    (i32.store (i32.const 1172) (i32.const 1024))
    (drop (call $fd_read
      (i32.load (i32.const 1160)) (i32.const 1168) (i32.const 1) (i32.const 1152)))
    (drop (call $fd_close (i32.load (i32.const 1160))))
    (local.set $i
      (i32.sub (i32.add (i32.const 128) (i32.load (i32.const 1152))) (i32.const 1)))
    (loop $scan
      (if (i32.lt_s (local.get $i) (i32.const 128)) (then (unreachable)))
      (local.set $digit (i32.load8_u (local.get $i)))
      (local.set $i (i32.sub (local.get $i) (i32.const 1)))
      (br_if $scan
        (i32.or
          (i32.lt_u (local.get $digit) (i32.const 48))
          (i32.gt_u (local.get $digit) (i32.const 57)))))
    (i32.store8 (i32.const 73) (i32.add (local.get $digit) (i32.const {delta})))
    (i32.store (i32.const 1160) (i32.const 0))
    (drop (call $path_open
      (i32.const 3) (i32.const 0) (i32.const 32) (i32.const {out_len})
      (i32.const 9) (i64.const 64) (i64.const 0) (i32.const 0) (i32.const 1160)))
    (i32.store (i32.const 1184) (i32.const 64))
    (i32.store (i32.const 1188) (i32.const 11))
    (drop (call $fd_write (i32.load (i32.const 1160)) (i32.const 1184) (i32.const 1) (i32.const 1192)))
    (drop (call $fd_close (i32.load (i32.const 1160))))
    (drop (call $fd_write (i32.const 1) (i32.const 1184) (i32.const 1) (i32.const 1192)))))"#,
        ctx_len = ctx_path.len(),
        out_len = out_path.len(),
    )
}

#[tokio::test]
async fn diamond_workflow_wasm_steps_feed_the_agent_step() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let data_dir = fleet.root().join("n0/node");
    stage_module(&data_dir, "echo.wasm", &echo_wat());
    stage_module(&data_dir, "plus1.wasm", &delta_wat("b", 1));
    stage_module(&data_dir, "plus3.wasm", &delta_wat("c", 3));
    // The agent step answers with prose around a ```json fence so the run
    // loop recovers structured output via `extract_output_json_from`.
    client.queue_script(completed(
        "summary ready\n```json\n{\"total\":6,\"from\":{\"b\":2,\"c\":4}}\n```\n".to_string(),
    ));
    let saved = fleet
        .call(
            "POST",
            "/api/dag/defs",
            json!({"spec":{"name":"diamond","steps":[
                {"name":"a","kind":{"type":"wasm","command":"echo.wasm"}},
                {"name":"b","depends_on":["a"],"kind":{"type":"wasm","command":"plus1.wasm"}},
                {"name":"c","depends_on":["a"],"kind":{"type":"wasm","command":"plus3.wasm"}},
                {"name":"d","depends_on":["b","c"],
                 "kind":{"type":"agent","prompt":"汇总 b 与 c 的结果"}}
            ]}}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let dispatched = fleet
        .call(
            "POST",
            "/api/dag/defs/diamond/dispatch",
            json!({"id":"dag-diamond-1"}),
        )
        .await;
    assert_eq!(dispatched.status, 202, "{dispatched:?}");
    let detail = settled(&fleet.nodes[0], "dag-diamond-1").await;
    assert_eq!(detail["execution"]["status"], "done", "{detail}");

    // 1. Step artifacts: the wasm modules really computed a → b/c and the
    //    agent step recovered the fenced summary as its output.json.
    let run = fleet.root().join("n0/node/dag/dag-diamond-1");
    let output = |step: &str| -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(run.join(step).join("output.json"))
                .unwrap_or_else(|e| panic!("{step}/output.json missing: {e}")),
        )
        .unwrap()
    };
    assert_eq!(output("a"), json!({"value":1}));
    assert_eq!(output("b"), json!({"value":2}));
    assert_eq!(output("c"), json!({"value":4}));
    assert_eq!(output("d"), json!({"total":6,"from":{"b":2,"c":4}}));

    // 2. The wasm steps really READ the upstream context: b's context.json
    //    carries a's structured output plus its success flag.
    let b_ctx: Value = serde_json::from_str(
        &std::fs::read_to_string(run.join("b").join("context.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(b_ctx["steps"]["a"]["json"], json!({"value":1}));
    assert_eq!(b_ctx["steps"]["a"]["ok"], json!(true));

    // 3. The agent step really received a/b/c outputs: its user prompt
    //    embeds the pretty upstream context (`a` is a TRANSITIVE upstream
    //    of `d`, so all three values must appear in the same prompt).
    let requests = client.requests();
    let agent_prompt = requests
        .iter()
        .flat_map(|request| request.messages.iter())
        .map(|message| message.text())
        .find(|content| content.contains("汇总 b 与 c 的结果"))
        .expect("agent step prompt request missing");
    for fragment in ["\"value\": 1", "\"value\": 2", "\"value\": 4"] {
        assert!(
            agent_prompt.contains(fragment),
            "prompt misses upstream {fragment}: {agent_prompt}"
        );
    }
    fleet.shutdown().await;
}
