# 🚀 快速上手

本文面向新用户：如何提交任务、用 notepad 浏览/编辑文件、以及给任务附加备注。命令保持英文原文，界面键位以 TUI 内实际显示为准。

## 📝 提交任务

### 交互式 TUI

直接运行 `opencoder` 进入 TUI：

- **`Enter`** — 提交输入框内容
- **`Tab`** — 运行中提交追问（入队为 followup，在 turn 边界插入；空闲时等价于普通提交）
- **`/plan <内容>`** — 复合提交：切换到 plan 模式并携带内容，例如 `/plan 实现一个 LRU cache`
- **`/task`** — 打开会话选择器，切换/恢复历史会话

### 无头运行

```bash
# 一次性运行，输出到 stdout（两种写法等价）
opencoder "用 Rust 实现一个 LRU cache 并写测试"
opencoder run "用 Rust 实现一个 LRU cache 并写测试"
```

### 全局 flag（TUI 与无头均可用）

```bash
opencoder --continue              # 恢复当前 workdir 最近会话
opencoder --session <id> "继续"    # 恢复指定会话
opencoder --fork "继续"            # 恢复前复制会话，原会话保持不变
opencoder --model anthropic/claude-3 "..."   # 覆盖模型（{provider}/{model_id}）
opencoder --image screenshot.png "看看这个截图"  # 附带图片（需 vision 模型）
```

### 远程与 tmux

```bash
# 远程：一台机器起 server，另一台用 client 接入
opencoder server --host 0.0.0.0 --port 8080
opencoder client --remote http://127.0.0.1:8080 "总结这个仓库的架构"

# tmux：SSH 断线后会话存活，重连后自动 reattach
opencoder ts          # 新建/恢复 tmux 会话
opencoder ts -l       # 列出受管会话（tmux 名 + 任务信息）
opencoder ts -r <id>  # 恢复指定会话
```

## 📓 Notepad：文件浏览与编辑

`/notepad`（别名 `/note`）打开全屏 IDE：左侧文件树 + 右侧 vim 编辑器，聊天区暂时隐藏。**Notepad 是纯内存的文件浏览/编辑视图，不会把文件内容注入模型上下文**。

### 文件树

| 按键 | 作用 |
| --- | --- |
| `j` / `k` | 上下移动 |
| `Enter` / `l` / `→` | 打开文件（或展开目录） |
| `n` | 新建文件 |
| `d` | 删除文件（需确认） |
| `H` | 隐藏/显示文件树 |
| `/` | 内容搜索 |

### vim 编辑器

| 按键 | 作用 |
| --- | --- |
| `i` / `a` | 进入插入模式 |
| `Esc` | 返回普通模式 |
| `:w` | 保存（不退出） |
| `:wq` / `:x` | 保存并退出编辑器，回到文件树 |
| `:e <path>` | 打开指定文件（相对工作目录） |
| `u` / `Ctrl+R` | 撤销 / 重做 |
| `Ctrl+D` / `Ctrl+U` | 向下 / 向上翻半页 |
| `Ctrl+F` / `Ctrl+B` | 向下 / 向上翻页 |
| `/` | 内容搜索 |

### 搜索

`/` 打开搜索面板：输入关键字后 `Enter` 执行，`j`/`k` 选择结果，`Enter` 在编辑器中打开命中位置。搜索后端优先使用 **ripgrep（`rg`）**，不可用时回退到 `grep -rn`。

### 面板切换与退出

`Tab` 在文件树与编辑器之间切换焦点；`Esc` 退出 Notepad 回到聊天视图。编辑器内若有未保存修改，退出时会自动保存。

## 🏷️ Annotation：任务备注

`/annotation`（别名 `/ann`）打开 vim 编辑器编辑任务备注。备注保存在会话记录中，**跨 `--continue` / `--session` 恢复依然保留**。

| 按键 | 作用 |
| --- | --- |
| `:wq` | 保存并退出 |
| `:q!` / `:q` | 放弃修改并退出 |

> 注：插入模式下 `Enter` 插入换行、`Ctrl+C` 返回普通模式；它们不再是退出路径，需用 `:wq` 或 `:q!`/`:q` 退出。

查看已保存的备注：

```bash
opencoder session show <id> --json
```
