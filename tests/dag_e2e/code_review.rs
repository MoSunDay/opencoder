//! M2 — code-review 发布门禁 DAG 全链路（进程级）。九步 spec 复刻
//! `examples/dag/code-review.json` 的拓扑（kb-index wasm → 范围 → 双锚定 →
//! 双审查 → 重核 → 裁决 → 工单），两条断言线：干净变更全链 done、
//! verdict=pass、工单 `not_required`；双 P1 实锤 → verdict=blocked、
//! 工单建 2 张。同时钉住 M2 接线：compat dispatch `input` 透传进
//! `<node>/dag/<run>/input.json` 并以「执行要求」后缀注入每个 agent
//! prompt；knowledge_root 存在时 prompt 尾部带「知识库（只读）」提示。

use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::{LlmStub, Script, EXTRA_REPLY};
use serde_json::{json, Value};
use std::path::Path;

const DEF: &str = "code-review";
const BASE_SHA: &str = "1111111111111111111111111111111111111111";
const HEAD_SHA: &str = "2222222222222222222222222222222222222222";

/// 路由关键词：每步 prompt 的独占开头（stub 按请求体判别，不依赖顺序）。
const K_SCOPE: &str = "圈定变更范围";
const K_API_ANCHOR: &str = "锚定对外 API 行为面";
const K_HARNESS_ANCHOR: &str = "锚定回归护网";
const K_API_REVIEW: &str = "审查对外影响";
const K_HARNESS_REVIEW: &str = "审查护网影响";
const K_CONFIRM: &str = "逐条重核";
const K_SUMMARY: &str = "出口裁决";
const K_TICKET: &str = "创建工单";

const KB_OUTPUT: &str = r#"{"entries":2,"knowledge_root":"/workspace/knowledge","manifest":"manifest.json","truncated":false}"#;
const KB_MANIFEST: &str =
    r#"[{"path":"docs/api.md","size":12},{"path":"docs/runbook.md","size":9}]"#;

/// wat data 段字面量转义（`"` 与 `\`）。
fn wat_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// kb-index 的 wasm 步替身：以 preopen fd 3（运行根，可写）为基准，
/// 用相对路径写 `<step>/output.json` 与 `<step>/manifest.json`，再向
/// stdout 打一行摘要（被采集进 output.txt）。与真实
/// `examples/dag-modules/kb-index` 的产物契约一致。
fn kb_index_wat() -> String {
    let out_path = "kb-index/output.json";
    let man_path = "kb-index/manifest.json";
    let line =
        "{\"step\":\"kb-index\",\"entries\":2,\"truncated\":false,\"knowledge_root\":\"/workspace/knowledge\"}".to_string();
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_close"
    (func $fd_close (param i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 16) "{out_path}")
  (data (i32.const 64) "{out}")
  (data (i32.const 192) "{man_path}")
  (data (i32.const 240) "{man}")
  (data (i32.const 384) "{line}")
  (func $write (param $po i32) (param $pl i32) (param $do i32) (param $dl i32)
    (local $fd i32)
    (local.set $fd (i32.const -1))
    (if (i32.eqz (call $path_open
        (i32.const 3) (i32.const 0) (local.get $po) (local.get $pl)
        (i32.const 9) (i64.const 64) (i64.const 0) (i32.const 0) (i32.const 0)))
      (then
        (local.set $fd (i32.load (i32.const 0)))
        (i32.store (i32.const 8) (local.get $do))
        (i32.store (i32.const 12) (local.get $dl))
        (drop (call $fd_write (local.get $fd) (i32.const 8) (i32.const 1) (i32.const 4)))
        (drop (call $fd_close (local.get $fd))))))
  (func (export "_start")
    (call $write (i32.const 16) (i32.const {opl}) (i32.const 64) (i32.const {ol}))
    (call $write (i32.const 192) (i32.const {mpl}) (i32.const 240) (i32.const {ml}))
    (i32.store (i32.const 8) (i32.const 384))
    (i32.store (i32.const 12) (i32.const {ll}))
    (drop (call $fd_write (i32.const 1) (i32.const 8) (i32.const 1) (i32.const 4)))))"#,
        out_path = out_path,
        out = wat_escape(KB_OUTPUT),
        man_path = man_path,
        man = wat_escape(KB_MANIFEST),
        line = wat_escape(&line),
        opl = out_path.len(),
        ol = KB_OUTPUT.len(),
        mpl = man_path.len(),
        ml = KB_MANIFEST.len(),
        ll = line.len(),
    )
}

/// 与 examples/dag/code-review.json 相同拓扑的九步 spec（e2e 内联副本，
/// kb-index 走默认 in_process 沙箱以脱离 runc rootfs 依赖）。
fn spec() -> Value {
    let agent = |name: &str, prompt: &str, deps: Value, timeout: u64| {
        json!({"name": name, "depends_on": deps,
               "kind": {"type": "agent", "prompt": prompt}, "timeout_secs": timeout})
    };
    json!({
        "name": DEF,
        "description": "发布门禁：知识库 base..head 变更审查 e2e 副本",
        "steps": [
            {"name": "kb-index",
             "kind": {"type": "wasm", "command": "kb-index.wasm"},
             "timeout_secs": 120},
            agent("change-scope",
                  "圈定变更范围：读取 base/head 提交，diff 出变更文件并归组模块。输出契约：末尾 ```json 围栏 {\"changed_files\":[{\"path\":\"...\",\"add\":1,\"del\":0}],\"modules\":[\"...\"],\"base\":\"<sha>\",\"head\":\"<sha>\"}",
                  json!(["kb-index"]), 300),
            agent("api-anchor",
                  "锚定对外 API 行为面：定位变更触及的对外契约锚点。输出契约：末尾 ```json 围栏 {\"anchors\":[{\"api\":\"...\",\"files\":[\"...\"],\"rationale\":\"...\"}]}",
                  json!(["change-scope", "kb-index"]), 300),
            agent("harness-anchor",
                  "锚定回归护网：定位变更触及的回归测试锚点。输出契约：末尾 ```json 围栏 {\"anchors\":[{\"harness\":\"...\",\"files\":[\"...\"],\"rationale\":\"...\"}]}",
                  json!(["change-scope", "kb-index"]), 300),
            agent("api-review",
                  "审查对外影响：对照 API 锚点审查 base..head 变更。输出契约：末尾 ```json 围栏 {\"issues\":[{\"id\":\"API-1\",\"severity\":\"P2\",\"summary\":\"...\",\"evidence\":\"file:line\"}]}",
                  json!(["api-anchor"]), 300),
            agent("harness-review",
                  "审查护网影响：对照护网锚点审查 base..head 变更。输出契约：末尾 ```json 围栏 {\"issues\":[{\"id\":\"HAR-1\",\"severity\":\"P1\",\"summary\":\"...\",\"evidence\":\"file:line\"}]}",
                  json!(["harness-anchor"]), 300),
            agent("confirm",
                  "逐条重核：复核 api-review 与 harness-review 的每条问题。输出契约：末尾 ```json 围栏 {\"confirmed\":[\"HAR-1\"],\"rejected\":[],\"notes\":\"...\"}",
                  json!(["api-review", "harness-review"]), 300),
            agent("summary",
                  "出口裁决：聚合重核结果出发布门禁裁决。裁决规则：任一 P0/P1 实锤→blocked；仅 P2→pass_with_tickets；无实锤→pass。输出契约：末尾 ```json 围栏 {\"verdict\":\"pass|pass_with_tickets|blocked\",\"p0\":0,\"p1\":0,\"p2\":0,\"confirmed_issues\":[],\"report\":\"...\"}",
                  json!(["confirm"]), 300),
            agent("viking-ticket",
                  "创建工单：为实锤问题创建 viking 工单，幂等键 code-review:<head>:<序号>；无实锤输出 {\"tickets\":\"not_required\"}。输出契约：末尾 ```json 围栏 {\"tickets\":\"created|not_required\",\"ids\":[\"...\"]}",
                  json!(["summary"]), 300),
        ],
    })
}

fn fenced(payload: &str) -> String {
    format!("结论如下。\n```json\n{payload}\n```\n")
}

/// 按请求体关键词路由的 LLM 桩：`blocked` 切换审查/重核/裁决/工单四步
/// 的剧本，其余步骤两条线共用。
fn router(blocked: bool) -> Script {
    Script::dynamic(move |body| {
        let prompt = body["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default()
            .to_string();
        let reply = if prompt.contains(K_SCOPE) {
            fenced(&format!(
                r#"{{"changed_files":[{{"path":"src/api.rs","add":10,"del":2}}],"modules":["api"],"base":"{BASE_SHA}","head":"{HEAD_SHA}"}}"#
            ))
        } else if prompt.contains(K_API_ANCHOR) {
            fenced(
                r#"{"anchors":[{"api":"GET /v1/items","files":["src/api.rs"],"rationale":"响应形状变更"}]}"#,
            )
        } else if prompt.contains(K_HARNESS_ANCHOR) {
            fenced(
                r#"{"anchors":[{"harness":"tests/api_e2e","files":["tests/api_e2e/main.rs"],"rationale":"覆盖对外行为"}]}"#,
            )
        } else if prompt.contains(K_API_REVIEW) {
            fenced(if blocked {
                r#"{"issues":[{"id":"API-1","severity":"P1","summary":"错误码 400→404 属破坏性变更","evidence":"src/api.rs:88"}]}"#
            } else {
                r#"{"issues":[]}"#
            })
        } else if prompt.contains(K_HARNESS_REVIEW) {
            fenced(if blocked {
                r#"{"issues":[{"id":"HAR-1","severity":"P1","summary":"回归用例未覆盖新错误码","evidence":"tests/api_e2e/main.rs:12"}]}"#
            } else {
                r#"{"issues":[{"id":"HAR-2","severity":"P3","summary":"建议补充用例","evidence":"tests/api_e2e/main.rs:30"}]}"#
            })
        } else if prompt.contains(K_CONFIRM) {
            fenced(if blocked {
                r#"{"confirmed":["API-1","HAR-1"],"rejected":[],"notes":"两条均复现"}"#
            } else {
                r#"{"confirmed":[],"rejected":[{"id":"HAR-2","reason":"非行为面"}],"notes":"无实锤"}"#
            })
        } else if prompt.contains(K_SUMMARY) {
            fenced(if blocked {
                r#"{"verdict":"blocked","p0":0,"p1":2,"p2":0,"confirmed_issues":[{"id":"API-1","severity":"P1","summary":"错误码 400→404 属破坏性变更","evidence":"src/api.rs:88"},{"id":"HAR-1","severity":"P1","summary":"回归用例未覆盖新错误码","evidence":"tests/api_e2e/main.rs:12"}],"report":"两条 P1 实锤，拒绝放行"}"#
            } else {
                r#"{"verdict":"pass","p0":0,"p1":0,"p2":0,"confirmed_issues":[],"report":"无实锤问题，放行"}"#
            })
        } else if prompt.contains(K_TICKET) {
            fenced(if blocked {
                r#"{"tickets":"created","ids":["vg-101","vg-102"]}"#
            } else {
                r#"{"tickets":"not_required"}"#
            })
        } else {
            // 子会话标题 pass 等旁路请求：同一个 prompt 会再发一次并被
            // 上面命中；这里只兜底完全无关的请求。
            EXTRA_REPLY.to_string()
        };
        reply
    })
}

fn read_json_at(path: &Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn step_json(fleet: &Fleet, run: &str, step: &str) -> Value {
    read_json_at(
        &fleet
            .node_data
            .join("dag")
            .join(run)
            .join(step)
            .join("output.json"),
    )
}

struct Rig {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    stub: LlmStub,
    fleet: Fleet,
}

/// 公共脚手架：知识库目录 + wasm 池 + knowledge_root 配置 + kb-index
/// 发布 + spec 保存。返回可直接 dispatch 的 fleet。
fn rig(blocked: bool) -> Rig {
    // 8 个 agent 步 ×（主回合 + 标题 pass）+ 富余。
    let stub = LlmStub::spawn(vec![router(blocked); 24]);
    let tmp = tempfile::tempdir().unwrap();
    let kb = tmp.path().join("knowledge");
    std::fs::create_dir_all(kb.join("docs")).unwrap();
    std::fs::write(kb.join("docs/api.md"), "# api\n契约说明\n").unwrap();
    std::fs::write(kb.join("docs/runbook.md"), "# runbook\n").unwrap();
    let kb_abs = std::fs::canonicalize(&kb).unwrap();
    let fleet = Fleet::spawn_with_config(
        tmp.path(),
        stub.port(),
        json!({"dag": {
            "wasm_dir": tmp.path().join("wasm-pool"),
            "knowledge_root": kb_abs,
        }}),
        "cr-gate-node",
    );
    crate::fixtures::publish(&fleet, "kb-index", &kb_index_wat());
    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": spec()}));
    assert_eq!(status, 200, "save code-review def: {body}");
    Rig { tmp, stub, fleet }
}

/// 走 compat dispatch（带 input.prompt），返回 run id 并断言入队成功。
fn dispatch(fleet: &Fleet, run: &str) {
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{DEF}/dispatch"),
        &json!({"id": run, "input": {"prompt":
            format!("base={BASE_SHA} head={HEAD_SHA} 变更审查请求（发布门禁）")}}),
    );
    assert_eq!(status, 202, "dispatch {run}: {body}");
}

fn assert_gate_common(rig: &Rig, run: &str) {
    let fleet = &rig.fleet;
    let doc = fleet.wait_terminal(run);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");
    let (status, progress) =
        fleet.http("GET", &format!("/api/dag/runs/{run}/progress"), &json!({}));
    assert_eq!(status, 200);
    assert_eq!(progress["total"], 9, "progress: {progress}");
    assert_eq!(progress["done"], 9, "progress: {progress}");
    assert_eq!(progress["error"], 0, "progress: {progress}");

    // M2 接线一：compat dispatch 的 input 落到节点 input.json。
    let input = read_json_at(&fleet.node_data.join("dag").join(run).join("input.json"));
    assert_eq!(
        input["prompt"],
        json!(format!(
            "base={BASE_SHA} head={HEAD_SHA} 变更审查请求（发布门禁）"
        )),
        "input.json: {input}"
    );

    // M2 接线二：input 以「执行要求」后缀进入每个 agent prompt；
    // knowledge_root 存在时 prompt 尾部带「知识库（只读）」提示。
    let requests = rig.stub.wait_for_requests(8);
    let scoped: Vec<&String> = requests
        .iter()
        .filter(|r| r.contains(K_SCOPE) && r.contains("执行要求：base="))
        .collect();
    assert!(
        !scoped.is_empty(),
        "scope prompt 未带执行要求后缀: {requests:#?}"
    );
    assert!(
        scoped.iter().all(|r| r.contains("知识库（只读）")),
        "knowledge 提示未注入: {}",
        scoped[0]
    );

    // M2 接线三：kb-index 的 wasm 步产物契约（output.json + manifest.json）。
    assert_eq!(
        step_json(fleet, run, "kb-index"),
        serde_json::from_str::<Value>(KB_OUTPUT).unwrap(),
        "kb-index output.json"
    );
    let manifest = read_json_at(
        &fleet
            .node_data
            .join("dag")
            .join(run)
            .join("kb-index")
            .join("manifest.json"),
    );
    assert_eq!(
        manifest[0]["path"], "docs/api.md",
        "kb manifest: {manifest}"
    );
    for step in [
        "kb-index",
        "change-scope",
        "api-anchor",
        "harness-anchor",
        "api-review",
        "harness-review",
        "confirm",
        "summary",
        "viking-ticket",
    ] {
        let meta = read_json_at(
            &fleet
                .node_data
                .join("dag")
                .join(run)
                .join(step)
                .join("meta.json"),
        );
        assert_eq!(meta["outcome"], "done", "{step} meta: {meta}");
    }
}

#[test]
fn gate_passes_a_clean_change() {
    let rig = rig(false);
    let run = "dag-cr-e2e-pass-1";
    dispatch(&rig.fleet, run);
    assert_gate_common(&rig, run);

    // 裁决线：无实锤 → pass，且工单步不建单。
    let scope = step_json(&rig.fleet, run, "change-scope");
    assert_eq!(scope["base"], json!(BASE_SHA), "change-scope: {scope}");
    assert_eq!(scope["modules"][0], "api");
    let summary = step_json(&rig.fleet, run, "summary");
    assert_eq!(summary["verdict"], "pass", "summary: {summary}");
    assert_eq!(summary["p0"], 0);
    assert_eq!(
        summary["confirmed_issues"].as_array().map(Vec::len),
        Some(0)
    );
    let ticket = step_json(&rig.fleet, run, "viking-ticket");
    assert_eq!(ticket["tickets"], "not_required", "ticket: {ticket}");

    // summary 步经 HTTP API 读取：`output` 字段即 output.json。
    let (status, body) = rig.fleet.http(
        "GET",
        &format!("/api/dag/runs/{run}/steps/summary"),
        &json!({}),
    );
    assert_eq!(status, 200, "step summary: {body}");
    assert_eq!(body["output"]["verdict"], "pass", "step doc: {body}");
}

#[test]
fn gate_blocks_on_confirmed_p1_findings() {
    let rig = rig(true);
    let run = "dag-cr-e2e-block-1";
    dispatch(&rig.fleet, run);
    assert_gate_common(&rig, run);

    // 裁决线：两条 P1 实锤 → blocked，工单步逐条建单。
    let confirm = step_json(&rig.fleet, run, "confirm");
    assert_eq!(
        confirm["confirmed"].as_array().map(Vec::len),
        Some(2),
        "confirm: {confirm}"
    );
    let summary = step_json(&rig.fleet, run, "summary");
    assert_eq!(summary["verdict"], "blocked", "summary: {summary}");
    assert_eq!(summary["p1"], 2);
    assert_eq!(
        summary["confirmed_issues"].as_array().map(Vec::len),
        Some(2)
    );
    let ticket = step_json(&rig.fleet, run, "viking-ticket");
    assert_eq!(ticket["tickets"], "created", "ticket: {ticket}");
    assert_eq!(ticket["ids"].as_array().map(Vec::len), Some(2));
}
