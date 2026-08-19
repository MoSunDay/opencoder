Commit: (working-tree, post-7a9f188)

# 配置面与记忆注入修复批（非 core 侧）：CLI/web 模型覆盖守卫、api_key 脱敏双读面、AGENTS.md 注入确定性+200KB 上限、e2e 工程加固

> core 侧同批修复（`OPENAI_BASE_URL` 注册表同步、`redact_json` 本体、`api_key_for` 报错、`has_editable_key` 键集、`is_suspicious_model("")`、context-limit/capture warn、`skills_dir` Option 化）见同日 [config-hardening-env-overlay-redact-skills-home.md](config-hardening-env-overlay-redact-skills-home.md)。本文只记录 CLI/web/session/e2e 侧。

## 背景

六个分布在 CLI、web、session prompt、e2e harness 的缺陷：

1. `opencode config --help` 声称优先级是 `defaults < env < project file merged`——env vars 实际在整个文件链（含项目文件）之上，help 文案与真实解析顺序相反。
2. `config show`（CLI）与 `GET /api/config`（web）都原样回显合并后的 config JSON，provider `api_key` 全文泄漏到 stdout / HTTP 响应。
3. `--model` malformed（空串 / `x` / `ab/c`）在三个入口两条路径上表现不一：CLI headless 与 `todos run` 把坏值静默写进 config（每个请求都以坏 model id 失败，报错完全不指向 `--model`）；web `POST /api/prompt` 带 malformed `model` 字段同理，且报错被 api-key 解析失败遮蔽（后者先炸，用户看到的是 key 报错而非 model 报错）。
4. AGENTS.md 注入是 `read_dir` 取第一个匹配——文件系统顺序不定，同目录存在 `AGENTS.md` 与 `agents.MD` 等变体时选谁全凭 inode 顺序；且无大小上限，一个超大 AGENTS.md 会撑爆 system prompt。
5. e2e harness 有一批工程债：`_boot_serve` 对不退出的进程做阻塞 read（可能永久挂死）；`_free_port` 返回端口到真正 bind 之间有 TOCTOU 窗口；E8 bundle 用固定名临时文件；`seed_workdir` 泄漏 `/tmp/opencoder_e2e_*`；E19 对 live model 才能决定的 workflow 终态做 hard assert；默认二进制路径硬编码 `/data/caches/opencoder-target`（换机即坏）。

## 根因

- help 文案写于 env overlay 语义定稿前，未随解析链更新。
- 两个 JSON 读面各自序列化 `Config`，没有一个共享的脱敏点；core 缺脱敏器（本轮补 `redact_json`，见兄弟条目）。
- `--model` 覆盖发生在 `Config` 构建早期但无校验：坏值一路带进端点解析。校验必须放在**api-key 解析之前**，否则错误次序不可控。
- `find_agents_md` 依赖 `read_dir` 迭代顺序；无截断逻辑。
- e2e 是逐场景渐进长出来的，进程生命周期与资源清理缺少统一约定。

## 变更

### CLI

- **`crates/cli/src/lib.rs`** — `config` 子命令 doc 改为 `defaults < config files < env vars < --model`（与真实解析顺序一致）。
- **`crates/cli/src/model_override.rs`（新，150 行）** — `--model` 覆盖收敛为两个纯函数：`apply_model_override(&mut Config, &Option<String>) -> Result<bool, String>`（headless 新 run，返回 config 是否变更）与 `reapply_resume_model(&mut SessionState, &Option<String>) -> Result<Option<String>, String>`（resume 时显式 `--model` 胜过已存 model，返回新 model 由调用方持久化）。malformed 值返回 `Err`（错误串 `malformed --model value \`...\`: expected "provider/model" with each side at least 2 chars`），**绝不静默落 config**。`run.rs` 与 `todos_cmd.rs` 均以 `map_err` 传播为进程错误退出。
- **`crates/cli/src/session_cmd.rs`** — `config_show_json` 序列化前过 `opencoder_core::config::redact::redact_json`；stdout/stderr 行为不变（stdout 仍纯 JSON pretty，banner 仍在 stderr）。

### web

- **`crates/web/src/api_ops.rs`** — `apply_prompt_model`：prompt body 的 `model` 字段 malformed（空串/单边/过短）时在 drain 启动前返回 400（`invalid model \`...\`: malformed, expected "provider/model" ...`，措辞镜像 CLI）；模型校验**先于**端点/api-key 解析，坏 model 永远报 model 错而非 key 错。`GET /api/config` 响应过 `redact_json` 后再返回。

### session（AGENTS.md 记忆注入）

- **`crates/session/src/prompt.rs`** —
  - `find_agents_md` 确定性化：收集目录内全部大小写不敏感 `AGENTS.md` 匹配（仅文件），**精确名 `AGENTS.md` 优先**，其余按文件名字节序取最小——不再依赖 `read_dir` 顺序。
  - 新 `AGENTS_MD_MAX_BYTES = 200 * 1024` 上限：`truncate_bytes` 沿 UTF-8 char boundary 回退切头，`cap_instructions` 在超限时追加标记行 `[AGENTS.md truncated: original size N bytes exceeds 200KB limit]`，让模型知道上下文被截。
- **`crates/session/src/runner/llm_call.rs` / `skill_context.rs`** — 仅注释：对齐「system prompt 每次调用重建并重读磁盘 AGENTS.md、跨调用**非 byte-stable**、不享受 provider prefix-cache」的现实（skill 尾注入迁出 system prompt 的原始动机记录）。

### e2e harness（scripts/e2e/）

- **`config_scenarios.py`（新）** — 免 key 配置面契约 E20，`--only cli` / `--only web` 两种模式都跑（e2e_glm.py 无条件追加）：
  - E20a env 覆盖生效：`OPENCODER_MODEL` + `OPENAI_BASE_URL` 同时到达顶层 `provider.*` **与命名 provider 注册表项**（`providers[zhipuai-coding-plan].base_url` 钉死，防注册表同步回归）。
  - E20b api_key 掩码：`config show` 永不输出完整 key（`sk-e***` 形态）。
  - E20c envs 激活 banner：隔离 HOME 下 `active env: <name>` 在 stderr、stdout 保持纯 JSON、env 层 config.json 真实合入。
  - E20d malformed `--model` 拒绝：非零 rc、错误先于任何 API-key 要求（`x` 与 `""` 双样本）。
- **`web_scenarios.py`** — `_collect_bounded`（`communicate(timeout)` + kill 兜底，永不阻塞 read）；`_boot_serve` 启动失败（含端口 bind 竞争）换端口重试一次。
- **`cli_scenarios.py`** — E8 bundle 用 `tempfile.mkstemp` 唯一化 + `finally` 清理；E19 workflow 终态断言 hard→soft（live model 配合度决定），契约面（workflow_id / resume 幂等 / observe 链）仍 hard（E18/E2 守卫经核查已存在，未动）。
- **`lib.py`** — `seed_workdir` 全部 workdir 经 `atexit` 清理（`/tmp/opencoder_e2e_*` 不再泄漏）；`resolve_bin` 优先级统一为 显式参数 > `OPENCODER_BIN` > `CARGO_TARGET_DIR/release/opencoder` > 仓库 `target/release/opencoder`（删除 `/data/caches` 硬编码，`test_install.sh` 同步为 `OPENCODER_E2E_SOURCE` 缺省仓库路径）。

## 测试清单

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| CLI --model 覆盖拒绝 malformed | `apply_model_override_rejects_malformed_values` | `crates/cli/src/model_override.rs` |
| CLI resume --model 拒绝 malformed | `reapply_resume_model_rejects_malformed_values` | `crates/cli/src/model_override.rs` |
| CLI --model 正常覆盖生效 | `apply_model_override_sets_provider_model` | `crates/cli/src/model_override.rs` |
| CLI resume --model 胜过已存 model | `reapply_resume_model_overrides_stored_model` | `crates/cli/src/model_override.rs` |
| CLI config show api_key 掩码 | `config_show_json_masks_api_keys` | `crates/cli/src/session_cmd.rs` |
| web GET /api/config 掩码 | `get_config_masks_api_keys` | `crates/web/tests/web_api_ops.rs` |
| web malformed model 400 | `malformed_model_override_is_a_400` | `crates/web/tests/prompt_delivery_validation.rs` |
| model 错误先于且区别于 api-key 错误 | `model_error_precedes_and_differs_from_api_key_error` | `crates/web/tests/prompt_delivery_validation.rs` |
| AGENTS.md 精确名优先于变体 | `project_instructions_prefers_exact_agents_md_name_over_variants` | `crates/session/tests/prompt.rs` |
| 无精确名时取字节序最小变体 | `project_instructions_without_exact_name_picks_smallest_variant` | `crates/session/tests/prompt.rs` |
| 小文件不截断 | `project_instructions_small_file_not_truncated` | `crates/session/tests/prompt.rs` |
| 超 200KB char-boundary 截断+标记 | `project_instructions_truncated_past_200kb_with_boundary_safe_cut` | `crates/session/tests/prompt.rs` |
| 截断原样返回 | `truncate_bytes_returns_input_when_within_limit` | `crates/session/src/prompt.rs`（内嵌） |
| 截断落 char boundary | `truncate_bytes_cuts_on_char_boundary` | `crates/session/src/prompt.rs`（内嵌） |
| 上限内透传 | `cap_instructions_under_limit_passes_through` | `crates/session/src/prompt.rs`（内嵌） |
| 超限截断+标记行 | `cap_instructions_over_limit_truncates_with_marker` | `crates/session/src/prompt.rs`（内嵌） |
| e2e E20a–E20d 免 key 干跑 | 18/18 passed, 0 failed, 0 skipped（fresh `target/release` 二进制实测） | `scripts/e2e/config_scenarios.py` |
| e2e Python 语法 gate | `python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py` ✓ | — |

## Impact Surface

- CLI：`--model` malformed 从「静默坏 config + 下游莫名失败」变为立即报错退出（headless / resume / `todos run` 三处）；`config show` 输出中所有字符串 `api_key` 变为 `前4字符***`（≤4 字符全 `***`）。
- web：`POST /api/prompt` 带 malformed `model` → 400 且文案点名该值；`GET /api/config` 响应 api_key 掩码。
- session：>200KB 的 AGENTS.md 注入被截断并带标记行（此前全量注入）；同目录多 AGENTS.md 变体时选择确定（精确名 > 字节序最小）。
- e2e：`/data/caches` 机器绑定解除（`OPENCODER_BIN`/`CARGO_TARGET_DIR` 可覆盖）；`/tmp` workdir 自动清理；serve 启动挂死/端口竞争不再卡死套件。
- 行数：`model_override.rs` 150（≤400）、`config_scenarios.py` 221、`prompt.rs` 393、`api_ops.rs` 291（迭代 ≤800）。

## 回归 gate

- `cargo test --workspace`：✅ 3100 passed / 0 failed（RC=0，2026-08-19 实跑，197 个测试二进制）。
- `cargo clippy --workspace --all-targets -- -D warnings`：✅ 0 警告 0 错误（2026-08-19 实跑）。
- `cargo build --workspace`：✅ 提交前实跑干净（dev profile，2026-08-19）。
- e2e 免 key 干跑：`python3 scripts/e2e/config_scenarios.py` 18/18（已实测）；`python3 -m py_compile scripts/e2e/*.py scripts/e2e_glm.py` ✓
