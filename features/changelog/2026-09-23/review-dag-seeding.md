Commit: c60e2162be48102badf53d8b97e7cfa030b59605

# 审查 DAG 启动初始化收敛

启动时只补种 review-harness-quick；移除 review-code-quick 和 review-full-acceptance，避免操作者删除后重启重新注册。保留按名跳过策略，已有自定义定义不覆盖。

3 个初始化测试覆盖首次仅生成保留项、重复启动幂等和操作者编辑保留。源码修改尚未发布到当前在线 Server，不能将在线定义删除等同于重启后不会恢复。

逻辑见 [control 索引](../../../agents/control/index.md)与 [seed_dags.rs](../../../crates/control/src/seed_dags.rs)。
