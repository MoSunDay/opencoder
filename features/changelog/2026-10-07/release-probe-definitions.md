# 发布验收持续使用原生探针定义

真实发布验收把固定的 DAG 探针定义传给发布后观察；信号回滚与再次发布分别生成任务 ID，保持探针定义不变。信号失败演练显式要求配套 rootfs，交给隔离环境验证。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| 观察阶段提交固定原生定义并保存记录 | `test_observation_submits_the_frozen_native_definition` | `scripts/acceptance/smooth_release/tests/test_live_probes.py` |
| 受理不可用时不写成功采样 | `test_observation_rejects_unavailable_admission_without_a_success_sample` | 同上 |
| 回滚、再发布使用不同 ID 和同一探针定义 | `test_signal_roundtrip_keeps_definitions_separate_from_submission_ids` | 同上 |

`python3 -B -m unittest discover -s scripts/acceptance/smooth_release/tests -v`：15 项通过。
