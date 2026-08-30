Commit: (working-tree, skill run-end 清除 + [active skill] tail fallback-only + bash_guard 换壳 shellguard)

# bash_guard 换壳：启发式分类器收敛为 shellguard 独立 crate（rippy 衍生，AST 级判定）

## 背景

旧 `bash_guard` 是手写启发式：正则/前缀匹配检测写操作（重定向、变更命令、包管理器、git 写）。结构性弱点：对包裹命令、命令替换、控制流复合体的判定靠穷举模式，`eval|source` 只查一层剥壳，语义等价变体（`python -c`、`perl -e`、SQL 客户端 heredoc 等）靠逐个补丁维护。本轮将分类核替换为独立 crate `opencoder-shellguard`——从 [rippy](https://github.com/mpecan/rippy)（MIT）提取的 shell 命令安全分类器，按 sandbox 策略改造：判定基于 rable 解析出的 AST，而非字符串形状。

## 变更

### 新 crate：`opencoder-shellguard`（`crates/shellguard/`）
- **策略**：一切携带风险的写 → 拦截；释放集仅 `/dev/null` 与 `/tmp`；**cwd/项目目录不释放**；`Allow` 放行，`Ask`/`Deny` 一律拦截；不可解析命令 fail-closed（`lib.rs::classify`）。
- **管线**：`nesting`（输入形状界定）→ `parser`（rable → AST）→ `ast`（节点分类）→ `resolve`（shell 展开静态求值）→ `analyzer`（逐命令下钻 `handlers` 注册表）；另有 `node_safety`/`perl_safety`/`python_safety`/`ruby_safety`/`sql` 解释器与 SQL 写面专项。
- **crate 级 lint**：`unwrap_used`/`expect_used`/`panic` 全部 `deny`——分类器自身不允许 panic 路径（`crates/shellguard/Cargo.toml`）。
- 最大文件 `src/ast.rs` 398 行 ≤ 400；workspace 成员与 `opencoder-shellguard.workspace` 依赖接入（`Cargo.toml`、`crates/session/Cargo.toml`、`Cargo.lock` 新增 `rable 0.1.15` 等）。

### session 侧适配（`crates/session/src/bash_guard.rs`，788 → 235 行）
- `classify(cmd)` 改为薄适配：转发 `opencoder_shellguard::classify` 并映射 `Verdict → BashVerdict`（`Allow → ReadOnly`；`Ask`/`Deny → WriteBlocked`，`bash_guard.rs:28`）。
- 命令解析助手 `cmd_base`/`strip_wrappers`/`strip_leading_sudo` **原样保留**——`tools::ssh_pty` 仍复用它们剥远端提权前缀（`bash_guard.rs:12`）。
- **sandbox 语义契约同步收敛**（`prompt.rs:227`、`agent.rs:165`）：`IN_SANDBOX_MODE` 环境块与 `SANDBOX_SUFFIX` 显式声明释放集（`/tmp` + `/dev/null`）且声明项目目录不可写；sandbox base prompt 不再提及 `question` 工具（澄清协议只存在于 task-plan skill body）。

### 兼容性 corpus（防判定回归）
- **`bash_guard_compat_tests.rs` + `bash_guard_compat_tests2.rs`**：共 229 行 / 230 断言表驱动用例（1 行双断言；22 个 compat 测试函数，另 8 个适配层 smoke），逐条对照旧启发式的预期判定，分歧按四类显式标注（行级归属：行尾标注优先，其次归入同数组内最近的组注释，否则 KEEP）——`RELEASE`（仅因写目标在释放集而翻转，16 断言）、`RETARGETED`（结构危险但目标碰巧在 /tmp，改指向非释放路径保持结构可证，35 断言）、`OVER-BLOCK (safe)`（新分类器更严：未知命令/sudo/pip/apt/make 等 fail-closed 拦截，13 断言）、`RELAXED (verified safe)`（旧误拦、新放行：`bash -c 'echo hi'` 等纯只读 payload 递归判定后无写面，7 断言）。两文件经 `#[path]` 挂入 `bash_guard.rs` 测试模块。

### 兼容审计修复：in-place 编辑逃逸（rippy 上游继承洞）
230 断言 corpus 审计发现 4 条旧拦新放的**真洞**（写目标非释放集，违反「blocked→allowed 仅限释放集」不变式），根因均为 rippy 上游缺陷、随移植继承：
- `sed --in-place` / `--in-place=.bak` / `-i.bak`（GNU 长形式与粘连短旗标后缀）：sed 处理器只精确匹配 `-i`。
- `perl -pi -e` / `-ipe` / `-i.bak -pe`、`ruby -pi -e`：聚合短旗标簇中的 `-i` 从未被识别。
修复（`crates/shellguard/src/handlers/`）：`sed.rs` 改用 `is_in_place_edit`（精确 `-i` + `has_glued_short_flag` + `has_flag_or_prefixed("--in-place")`）；新增共享助手 `args.rs::has_clustered_short_flag`（旗标簇逐字母扫描，遇非字母即停——正确处理 `-i.bak` 后缀与值旗标截断，大小写敏感故 `-Ilib` 不误伤），`perl.rs`/`ruby.rs` 在 `-e`/`-E` 采集**之前**先行检测。新增 10 个单测（每洞正反例），4 条判定翻转 allow→block（无既有 allow 断言受损）。

### 安全收敛：in-place 编辑逃逸洞（compat 审计发现，shellguard 修复）
「原拦仍拦」不变式逐条校验时抓到 rippy 上游继承洞——in-place 编辑命令在新分类器下从拦截翻为放行，违反非释放集 blocked→allowed 即 fail 的硬约束，已在 shellguard 修复（不可在 corpus 层容忍）：

- **sed**（`handlers/sed.rs`）：就地编辑判定原只匹配裸 `-i`，GNU 长形式 `--in-place`/`--in-place=.bak` 与胶合 `-i.bak` 全部漏放。新增 `is_in_place_edit`：`-i`（精确）∪ `-i<suffix>`（胶合）∪ `--in-place`（含 `=` 形式）→ Ask；`sed 's/a/b/' f` 等纯读仍放行。
- **perl / ruby**（`handlers/perl.rs`、`handlers/ruby.rs`）：簇合短旗标携带的 `-i`（`perl -pi -e`、`-ipe`、`-pi.bak -e`，ruby 同型）原被无视，`-e` 内嵌代码被判无害后放行。新增共享助手 `has_clustered_short_flag`（`handlers/args.rs`，簇内在首个非字母处截断以兼容备份后缀、大小写敏感防 `-I`/`-M` 误报、值旗标截断防参数值误判）→ Ask（`perl in-place edit (-i)` / `ruby in-place edit (-i)`）。
- 该组修复前曾以 `#[ignore]` 的 `compat_known_holes` 停车场保红可见；修复后转正为 `compat_in_place_edits_are_blocked` 并扩行（`sed -i.bak`、`ruby -pi -e`）。**当前无已知逃逸洞。**
- 新增 10 例 handler 单测（sed 6 形态 / perl 4 拦 + 5 放 / ruby 3 拦 + 3 放 / args 2 助手）；shellguard 360 → 368 例。

### 收敛记录（隔离复跑 + gate 总账）
- 全量 gate（静默机器 `cargo test --workspace --no-fail-fast`）：**3686 通过 / 0 失败**（239 个测试二进制全数产出 result 行，零启动崩溃；其中 shellguard 368、session 全量 1111 = lib 391 + 集成 720）。早期高负载 mega-run 曾报 4 具名失败（daemon/two_process/heartbeat 等 spawn 起动类，链接风暴 load>80 下超预算的争用假失败，隔离串行复跑均过：daemon 46.9s、two_process 58.9s、heartbeat 15.8s）与 52 个启动即崩二进制（SIGSEGV/127，非断言失败），静默机器复跑无一复现。
- 顺手修复 gate 暴露的前次会话遗留断点（与本轮无关但挡 gate）：`tests/clear_context_skill_compound.rs` ① `pending_inputs` 单参调用未跟上两参 `Delivery` 签名（补 `Delivery::Queue`）；② fixture 注释声称「stale skill 已装载」但从未装载（`run_kickoff_then_compound` 增加 `stale_skill` 参数，用例 1 传 `STALE_BODY`、用例 3 传 `None`）——3/3 转绿。

- corpus 分歧终账（行级，与变更节四类同口径）：RETARGETED 35（原目标在 /tmp 的结构危险用例改指非释放路径，保「结构绕过必拦」）、RELEASE 16（仅因 /tmp + /dev/null 释放翻转：`compat_tmp_release_flips` 组注释覆盖 9 断言 + `find /tmp -delete`、`zsh -c 'touch /tmp/pwned'` 内层写等尾标 7 断言）、OVER-BLOCK 13（旧放行新拦截，fail-closed 方向，逐条注明理由）、RELAXED 7（旧误拦新放行，逐条论证无写面，如 `bash -c 'echo hi'` 递归内层）；其余 159 断言两侧判定一致（230 断言 = 分歧 71 + 一致 159）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 适配层判定映射 | `classify_*`（Allow→ReadOnly / Ask·Deny→WriteBlocked） | bash_guard.rs（compat corpus 两文件 229 行 / 230 断言用例） |
| crate 单测 | `analyzer_pipeline_tests` / `analyzer_sandbox_tests`（shellguard 全 crate 368 例） | crates/shellguard/src/ |
| 兼容 corpus | 229 行表驱动用例（230 断言）+ `compat_in_place_edits_are_blocked`（含 in-place 洞回归钉） | crates/session/src/bash_guard_compat_tests{,2}.rs |
| sandbox 集成 | sandbox 写命令 ToolEnd is_error 含 "Blocked in sandbox mode"、只读放行 | crates/session/tests/bash_guard_sandbox_mode.rs |
| sandbox 提示契约 | `sandbox_prompt_is_read_only_without_question`（不提 question、声明释放集） | crates/core/src/agent.rs |

- 全量回归（终账，第三轮）：`cargo test --workspace --no-fail-fast -j 4` → 239 个测试二进制全部 `0 failed`、**3686 passed / 0 failed**、`CARGO-EXIT=0`（本轮无 SIGSEGV/超时假失败，覆盖前两轮链接风暴期全部缺口）；clippy workspace 0 告警。前两轮互证：3082 与 3104 passed / 0 failed（当时 52 个链接风暴期启动崩溃二进制已逐一隔离串行复跑全绿）。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 告警
- 行数：`crates/shellguard/src/ast.rs` 398 ≤ 400；`bash_guard.rs` 235 ≤ 800

## Impact Surface
- sandbox 模式下 bash 拦截更准：`/tmp`、`/dev/null` 写放行，其余风险写（含解释器内嵌代码、SQL、命令替换内的写）AST 级拦截；少数旧误拦场景转为放行（corpus 中 `RELAXED` 类逐条论证）。
- 不影响：act 模式（不设防）、`ssh_pty` 解析助手、工具注册面、store/web 边界。

## Related Docs
- [agents/shellguard](../../agents/shellguard/index.md)
- [既有相关 changelog](../2026-08-30/skill-run-end-clear-and-fallback-tail.md)
