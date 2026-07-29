Commit: (working-tree, pre-initial-commit)

# feat(tui): /config 表单新增 ctx size（context_limit）字段 + threshold 约束校验

## 背景

`/config` 生成参数表单（`ConfigForm`）此前可调 `context_threshold`（压缩触发点），
但缺少与之配对的 `context_limit`（上下文窗口大小）。用户无法在 TUI 内设定窗口大小，
且 threshold 与 context size 之间无约束关系——可把压缩阈值设到超过窗口大小，语义非法。

## 变更

- **`model_menu/config_form.rs`**
  - 新增 `ConfigField::ContextSize` variant，插入 `ORDER`（位于 `MaxTokens` 与
    `Threshold` 之间），表单字段导航链自动覆盖。
  - `ConfigForm` 新增 `context_size: u64` 字段，构造时由 `config.context_limit()`
    初始化（默认 128k / 配置自定义）。
  - `adjust_context_size(±1000)`：←/→ 步进 1k；`Char` 追加数字位；`Backspace` 弹出
    末位（`/ 10`，下限 1）。
  - `validate()` 新增约束：`threshold > context_size` 拒绝保存，返回错误信息。
  - 顺带补全 `Threshold` / `Fps` / `ApMaxIter` 的 Backspace 处理（此前缺失，统一交互）。
- **`model_menu/patch.rs`**：`ConfigPatch` 新增 `context_limit: u64` 字段，`to_json()`
  写入顶层 `context_limit`，与 `Config::context_limit()` 读取路径一致。
- **`model_menu/view.rs`**：渲染 `ctx size:` 字段行（`N tokens (≈Nk)` 提示），位于
  `ctx threshold:` 上方；同步补全各数字字段的 `Backspace` 提示文案。
- **`model_menu/tests/`**：新增 `backspace()` helper 与 6 个测试。

## 设计要点

- `context_limit` 为 `Config` 既有字段（`Option<u64>`），`context_limit()` 读取时
  缺省回退默认窗口。patch 写顶层 `context_limit`，与现有序列化路径对齐，不引入新键。
- `threshold > context_size` 校验只在 `validate()`（Save 时调用）阻断，不影响合法路径
  与正常字段编辑（用户仍可临时填入再回退）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 表单从 config 初始化 ctx size（默认 + 自定义） | `config_form_inits_context_size_from_config` | `tests/config_tests.rs` |
| 输入数字位设置 ctx size | `typing_digits_sets_context_size` | `tests/config_tests.rs` |
| Backspace 弹出 threshold 末位 | `backspace_pops_digit_from_threshold` | `tests/config_tests.rs` |
| Backspace 弹出 ctx size 末位 | `backspace_pops_digit_from_context_size` | `tests/config_tests.rs` |
| threshold > ctx size 时阻断保存 | `validate_rejects_threshold_above_context_size` | `tests/config_tests.rs` |
| patch 序列化写入 context_limit | `config_patch_writes_context_limit` | `tests/config_tests.rs` |

全部为内联单元测试，纯函数调用，无 I/O / 网络 / DB。

### 全量回归

| 检查 | 结果 |
|------|------|
| `cargo build --workspace` | PASS — Finished（隔离 worktree @ HEAD） |
| `cargo test --workspace` | PASS — **1272 passed / 0 failed**（隔离 worktree 于 HEAD `59a8f7e` + 本特性实测，含本轮 6 个新测试） |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告（隔离 worktree @ HEAD） |

防修绿扫描：无新增 `#[ignore]`、无删除测试、无弱断言、无调试输出。（本提交已与并发背景改动干净分离：ap_skill 链路完整保留，仅新增 context_size。）

## Impact Surface

- `/config` 表单新增 ctx size 字段，可设上下文窗口；保存时 threshold 不得超过 ctx size。
- `ConfigPatch` 序列化新增顶层 `context_limit` 键。
- 不影响：drain 语义 / Store / ChatStream / runner / web / cli。改动隔离于 TUI 表单层。
- 范围外脏改动（`chat.rs` 箭头图标）已识别并排除，不随本次提交。

## 风险与回退

- 低风险：新增字段为增量式（新 enum variant / 新 struct field），不改变既有字段语义。
- 回退：删除 `ContextSize` variant / `context_size` 字段 / `context_limit` patch 字段即可。
