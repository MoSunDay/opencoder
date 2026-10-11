Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# Codex 宿主代理启动入口

宿主会话使用的 Codex 配置可能引用已退出的临时代理，导致启动失败。启动入口每轮发现节点现有代理并建立随子进程退出的桥接，保留参数、提示词标准输入与退出码。代理不可用时阻止启动。代理发现模块由执行节点运行库提供，以 `_socks5.py` 后缀识别；缺少或存在多个模块时拒绝启动。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| 参数与提示词标准输入保留 | `test_preserves_stdin_and_literal_arguments` | [test_codex_proxy.py](../../../scripts/harness/test_codex_proxy.py) |
| 代理不可用时阻止启动 | `test_missing_proxy_fails_before_codex_starts` | [test_codex_proxy.py](../../../scripts/harness/test_codex_proxy.py) |

| 代理模块缺失或歧义时拒绝启动 | `test_ambiguous_or_missing_provider_fails_before_executor_loads` | [test_codex_proxy.py](../../../scripts/harness/test_codex_proxy.py) |

宿主入口定向回归 3 项通过；子进程退出后桥接端口关闭。相关索引：[Session](../../../agents/session/index.md)、[Web](../../../agents/web/index.md)。
