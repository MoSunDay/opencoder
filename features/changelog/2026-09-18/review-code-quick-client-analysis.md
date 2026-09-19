# review-code-quick 增加客户端代码评审（API 影响面 → 客户端分支）

## 背景

`review-code-quick` 此前是四步线性链（triage→risks→verdict→report），只评审服务端风险，
缺少对客户端代码的分析：API 变更若未同步评审消费方客户端代码，可能出现输入未提交被清空、
点击交互失效、无法滑动等 UI 交互体验受损的问题而不被门禁捕获。

## 变更

- `crates/control/src/seed_dags.rs`：`spec_code_quick` 从 4 步扩为 6 步带分支——
  - 新增 `review-api-impact`（dep triage）：从上游范围归纳 API 影响面，输出
    `{"api_impacts":[{"api","change","impact"}]}`；
  - 新增 `review-client`（dep api-impact）：先据 API 影响面关联客户端设计范围
    （页面/组件/交互流），再按"UI 交互体验不受损、符合正常交互逻辑"标准评审客户端代码，
    问题按 P0（业务逻辑阻断）/P1（业务逻辑受损）/P2（影响体验）定级，输出
    `{"client_scope":[...],"issues":[{"severity","surface","why"}]}`；
  - `review-verdict` 依赖改为 `["review-risks","review-client"]`，汇入客户端评审结论
    （客户端 P0/P1 计入阻断）。
- `tests/dag_e2e/review_dags/mod.rs`：
  - R1 断言 6 步 + 分支拓扑（verdict 双依赖）；
  - R4 stub router 增加客户端/API 影响面两个路由键（顺序：报告→结论→客户端→API→风险→兜底），
    steps 产物断言补 `api_impacts`/`client_scope`/`issues`，请求桩扩到 6 步；
  - R4 上下文断言补三条传递链：api_impacts→client prompt、client_scope→verdict prompt、
    triage 经传递祖先同时到达 client 与 verdict prompt（验证 `render_context` 祖先注入语义）。

## 测试

- `cargo test -p opencoder-control --lib seed_dags`：3 过。
- `cargo test --test dag_e2e review_dags -- --test-threads=1`：R1–R5 共 5 过。
- `cargo test --test dag_e2e -- --test-threads=1`：全量 14 过。
- `cargo clippy -p opencoder-control -p opencoder --tests`：无告警。
