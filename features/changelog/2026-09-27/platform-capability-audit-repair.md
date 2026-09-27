Commit: 2aa44247d199d782881b9ee64921c4c6de2e6199

# 平台盘点修复回执（repair）

对 proof 确认的 P1/P2/P10 完成修复核对：单执行者只读核对在途 diff、复扫并产出回执，未改写仓库（并发保护：HEAD 已从 `6b5b4d28` 前进到 `2aa44247`，工作区 47 文件在途，修复面 5 文件与其余并发改动无交叉，未触碰）。逐项 diff、责任模块与证据在产物 05：
`/data00/workspace/artifacts/opencoder-platform-audit/brain-opencoder-platform-audit-20260927/`。

- P1 文档子命令：README×2 + quickstart×2 已改 `opencoder-server`/`opencoder-agent`/`opencoder-cli`，全 README/docs 复扫旧命令 0 命中（判据）。
- P2 架构描述：README.md 改为依赖分层表述、无 crate 计数，`Cargo.toml` members 实数 23 对齐（判据）。
- P10 daemon token 回显：`migration_hint` 不再拼接 token、统一提示 `--token-file`，单测负向断言（`server_hint_omits_token_and_preserves_web_off` / `client_hint_carries_remote_name_without_token`，定向测试 8 passed / 0 failed，根执行者回执；复现仅用无效哨兵值，严禁真实凭据）。
- 责任模块：入口文档（README*、docs/quickstart*）与 `crates/local`；仓库无权威 CODEOWNERS，个人负责人待确认。

范围外维持原判：P3/P5/P8 仅维护性事实记录，P4/P6/P7/P9 已排除，不再重复排查。

## 相关

- [实锤验证（proof）](platform-capability-audit-proof.md)
- [盘点（01–03 产物）](platform-capability-audit.md)
- [local 模块](../../../agents/local/index.md)
