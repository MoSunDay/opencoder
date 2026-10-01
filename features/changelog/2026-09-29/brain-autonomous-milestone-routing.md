Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 大脑自主选择里程碑路径

## 背景

新建计划要求人为绘制层间连线并填写决策条件，限制了大脑依据执行证据选择整改层的能力。

## 变更

- 计划画布只配置有序里程碑和各层能力；相邻层的线表示正常前进顺序，不再编辑决策条件或回退线。
- 大脑在层屏障后结合完整计划、绑定能力说明与执行结果，达标时前进一层，需整改时可选择本层或任一已执行层，并记录评估与反思。
- 新保存的计划版本由服务端补齐内部前进与回退路径，以满足现有运行图校验和旧节点重试；历史版本的显式路径保留。路径不作为模型决策条件。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 路径补齐与历史版本保留 | `rollback_paths_cover_every_executed_layer_without_changing_old_versions` | `crates/core/src/brain/layered/plan.rs` |
| 保存、幂等重试及旧节点准入 | `new_plan_save_is_idempotent_and_keeps_paths_for_retained_runtimes` | `crates/control/tests/e2e/layered_api/plans.rs` |
| 直接运行冻结 | `inline_plan_defaults_and_run_overrides_are_frozen_for_dispatch` | `crates/control/tests/e2e/layered_api/plans.rs` |
| 画布不编辑决策条件 | `在真实画布编辑节点后，表单提交保存新版本且不会启动运行` | `crates/web/spa/src/brain/workbench/milestone/editor.dom.test.jsx` |

- 全量回归：`cargo test --workspace -j8` → 430 个套件、5650 passed / 0 failed / 8 ignored。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。

## 相关

[当前工作台规则](../../brain/index.md) · [运行协议](../../../docs/brain-orchestration.md)
