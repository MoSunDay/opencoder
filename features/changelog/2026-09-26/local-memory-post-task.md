Commit: c60e2162be48102badf53d8b97e7cfa030b59605

# 完成后本地记忆维护

## 背景

手动在主任务上下文继续执行记忆更新会占用原会话历史；任务完成与记忆维护也没有统一的开关和结束边界。

## 变化

- `/config` 增加默认关闭的 `local-memory` 开关，保存为顶层 `local_memory`。
- 主任务成功完成后，在无会话存储的上下文副本中运行内置 `repo-local-memory`；维护完成后再发送任务结束事件，维护错误直接呈现。
- TUI 隐藏并拦截 Agent 命令，相关入口暂不开放。

## 影响

影响 Config 加载与保存、session 运行结束路径和 TUI 命令入口。详情见 [本地仓库记忆](../../local-memory/index.md)。
